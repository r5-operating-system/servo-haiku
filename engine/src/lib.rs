// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 R5OS Contributors
//
// Servo itself is MPL-2.0. This file is the R5 wrapper.
// Event numbers match distro/servo/r5_servo_abi.h.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};

use servo::{
    AuthenticationRequest, CreateNewWebViewRequest, Cursor, DeviceIntPoint, DeviceIntRect,
    DeviceIntSize, DevicePoint, EmbedderControl, EventLoopWaker, InputEvent,
    Key, KeyState,
 KeyboardEvent, LoadStatus, Modifiers, MouseButton,
    MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NamedKey, Opts, PermissionRequest,
    Preferences, RenderingContext, ServoBuilder, SimpleDialog, WebView, WebViewBuilder,
    WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};

static PROTOCOL_MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static LOGGER_ONCE: Once = Once::new();
// The map only needs the documents of open tabs. Past this many entries it
// starts over, so a long session does not grow it without end.
const PROTOCOL_MAP_LIMIT: usize = 2048;

type WakeFn = extern "C" fn(*mut std::ffi::c_void);

// Servo allows one instance per process (servo_config's OPTIONS is a
// OnceLock). Aomi runs one browser window per process, and every Servo call
// happens on that window's thread, so the instance lives in a thread local.
thread_local! {
    static SHARED_SERVO: RefCell<Option<Rc<servo::Servo>>> = const { RefCell::new(None) };
    // Engines alive on this thread. An event or JavaScript result for an
    // engine that is no longer listed is dropped.
    static ENGINES: RefCell<Vec<(usize, u64)>> = const { RefCell::new(Vec::new()) };
    // Set by r5_servo_shutdown(): Servo cannot be built twice in a process.
    static SHUT_DOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // Rendering contexts of closed tabs. Destroying one aborts in Haiku
    // Mesa's eglDestroyImageKHR, so a new tab reuses them instead.
    static CONTEXT_POOL: RefCell<Vec<(Rc<WindowRenderingContext>, i32, i32)>> =
        const { RefCell::new(Vec::new()) };
    // The size the GL storage of a new tab's surface is made with (see
    // surface_size()). r5_servo_set_max_size() sets it; 0 means unset.
    static MAX_SIZE: RefCell<(i32, i32)> = const { RefCell::new((0, 0)) };
    // The wake target is process-wide: see WAKE_TARGET below.
}

// The GL path. Servo's EGL is Mesa's surfaceless platform, where R5's own
// choice in the Haiku EGL driver (M9 decision 3) does not run. This makes the
// same choice as that driver and r5-gl-test's zink_asked(): Zink on the GPU
// when the radeon_hd settings allow it and a render node under /dev/gpu
// reports R5_GPU_CAP_AVAILABLE, otherwise Zink on Lavapipe. Forcing Zink
// without a GPU made Mesa abort in eglInitialize (QEMU, 26 Nov 2026).
extern "C" {
    fn load_driver_settings(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn get_driver_boolean_parameter(
        handle: *mut std::ffi::c_void,
        key: *const std::ffi::c_char,
        unknown_value: bool,
        no_arg_value: bool,
    ) -> bool;
    fn unload_driver_settings(handle: *mut std::ffi::c_void) -> i32;
    fn ioctl(fd: i32, op: u64, ...) -> i32;
}

fn gpu_settings_allow() -> bool {
    for path in [
        "/boot/home/config/settings/kernel/drivers/radeon_hd",
        "/boot/system/settings/kernel/drivers/radeon_hd",
    ] {
        if !std::path::Path::new(path).exists() {
            continue;
        }
        let Ok(name) = CString::new(path) else {
            return true;
        };
        let handle = unsafe { load_driver_settings(name.as_ptr()) };
        if handle.is_null() {
            return true;
        }
        let flag = |key: &str, default: bool| {
            let key = CString::new(key).unwrap_or_default();
            unsafe { get_driver_boolean_parameter(handle, key.as_ptr(), default, true) }
        };
        let allow = flag("gpu_opengl", true) && flag("gpu", true) && !flag("gpu_opengl_fail", false);
        unsafe { unload_driver_settings(handle) };
        return allow;
    }
    true
}

fn gpu_node_available() -> bool {
    use std::os::fd::AsRawFd;
    // R5 GPU interface (r5_gpu.h): GET_INFO is 30479; the first 32 bytes end
    // with the capabilities, and R5_GPU_CAP_AVAILABLE is 8.
    let Ok(drivers) = std::fs::read_dir("/dev/gpu") else {
        return false;
    };
    for driver in drivers.flatten() {
        let Ok(nodes) = std::fs::read_dir(driver.path()) else {
            continue;
        };
        for node in nodes.flatten() {
            let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(node.path())
            else {
                continue;
            };
            let mut info = [0u32; 8];
            info[0] = std::mem::size_of_val(&info) as u32;
            let result = unsafe {
                ioctl(
                    file.as_raw_fd(),
                    30479,
                    info.as_mut_ptr() as *mut std::ffi::c_void,
                    std::mem::size_of_val(&info),
                )
            };
            if result == 0 && info[7] & 8 != 0 {
                return true;
            }
        }
    }
    false
}

fn choose_gl_path() {
    // A program or the user that set either variable keeps its choice.
    if std::env::var_os("GALLIUM_DRIVER").is_some()
        || std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some()
    {
        return;
    }
    std::env::set_var("GALLIUM_DRIVER", "zink");
    if gpu_settings_allow() && gpu_node_available() {
        println!("r5-servo gl-path zink-gpu");
    } else {
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        println!("r5-servo gl-path zink-lavapipe");
    }
}

// Surfman's Mesa surfaceless backend makes a widget surface a GL surface of
// the size it is created with, and resize_surface() only records a new size
// (surfman mesa_surfaceless/device.rs). Making a new surface would mean
// destroying the old one, which aborts in Haiku Mesa's eglDestroyImageKHR.
// So the storage is made once, as large as the screen, and a view is never
// larger than its storage. WebRender draws with the recorded size from the
// lower left corner and read_to_image() reads from there, so a smaller
// recorded size inside larger storage is correct.
fn surface_size(width: i32, height: i32) -> (i32, i32) {
    let (max_w, max_h) = MAX_SIZE.with(|size| *size.borrow());
    (width.max(max_w).max(16), height.max(max_h).max(16))
}

/// The largest view this process will show, normally the screen size. Call
/// before r5_servo_create(); a tab's GL storage is made this large.
#[no_mangle]
pub extern "C" fn r5_servo_set_max_size(width: i32, height: i32) {
    MAX_SIZE.with(|size| *size.borrow_mut() = (width.max(0), height.max(0)));
}

// Process-wide: the C++ side may register the waker on another thread than
// the one that builds Servo (a view is attached on the app thread while its
// window is constructed).
static WAKE_TARGET: OnceLock<Arc<Mutex<Option<(WakeFn, usize)>>>> = OnceLock::new();
static NEXT_ENGINE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn wake_target() -> Arc<Mutex<Option<(WakeFn, usize)>>> {
    Arc::clone(WAKE_TARGET.get_or_init(|| Arc::new(Mutex::new(None))))
}

fn alive(engine: *mut Engine) -> bool {
    !engine.is_null()
        && ENGINES.with(|list| list.borrow().iter().any(|(ptr, _)| *ptr == engine as usize))
}

// The address of a freed engine can come back for a new tab; the id cannot.
fn alive_id(engine: *mut Engine, id: u64) -> bool {
    !engine.is_null()
        && ENGINES.with(|list| list.borrow().iter().any(|e| *e == (engine as usize, id)))
}

// Servo calls wake() from its own threads when it has work for the embedder.
// The C++ side then posts one message to the window, which calls
// r5_servo_pump() on the window thread.
struct Waker {
    target: Arc<Mutex<Option<(WakeFn, usize)>>>,
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(Waker {
            target: Arc::clone(&self.target),
        })
    }

    fn wake(&self) {
        let target = self.target.lock().ok().and_then(|slot| *slot);
        if let Some((function, cookie)) = target {
            function(cookie as *mut std::ffi::c_void);
        }
    }
}

fn protocol_map() -> &'static Mutex<HashMap<String, String>> {
    PROTOCOL_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

struct ProtocolLogger {
    verbose: bool,
}

impl log::Log for ProtocolLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target().contains("http_loader")
            || metadata.level() <= log::Level::Warn
            || self.verbose
    }

    fn log(&self, record: &log::Record) {
        if record.target().contains("http_loader") {
            let msg = format!("{}", record.args());
            let proto_and_url = if let Some(u) = msg.strip_prefix("HTTP/2.0 response for ") {
                Some(("h2", u.trim()))
            } else if let Some(u) = msg.strip_prefix("HTTP/3.0 response for ") {
                Some(("h3", u.trim()))
            } else if let Some(u) = msg.strip_prefix("HTTP/1.1 response for ") {
                Some(("http/1.1", u.trim()))
            } else if let Some(u) = msg.strip_prefix("HTTP/1.0 response for ") {
                Some(("http/1.0", u.trim()))
            } else {
                None
            };
            if let Some((proto, raw_url)) = proto_and_url {
                if let Ok(mut map) = protocol_map().lock() {
                    if map.len() >= PROTOCOL_MAP_LIMIT {
                        map.clear();
                    }
                    map.insert(shown_text(raw_url), proto.to_string());
                }
            }
            if self.verbose {
                eprintln!("[{} {}] {}", record.level(), record.target(), shown_text(&msg));
            }
        } else if self.verbose || record.level() <= log::Level::Warn {
            let msg = format!("{}", record.args());
            eprintln!("[{} {}] {}", record.level(), record.target(), shown_text(&msg));
        }
    }

    fn flush(&self) {}
}

fn install_protocol_logger() {
    LOGGER_ONCE.call_once(|| {
        let verbose = std::env::var("RUST_LOG").map(|v| !v.is_empty()).unwrap_or(false);
        let logger = Box::new(ProtocolLogger { verbose });
        if log::set_boxed_logger(logger).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
    });
}

// An address as Aomi shows and logs it: without the user name and password
// a sign-in put into it (Servo keeps them in the document's URL after an
// HTTP 401). Password material never goes into the address field, the
// history or a log.
// The same for any text that may hold an address (log lines, a link under
// the mouse): "scheme://user:password@host" loses "user:password@".
fn shown_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("://") {
        let (head, tail) = rest.split_at(start + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => rest = &tail[at + 1..],
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

fn shown_url(url: &url::Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.to_string();
    }
    let mut clean = url.clone();
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.to_string()
}

// Extra lines about engines, visibility and frames: only with R5_SERVO_DEBUG
// set (tests).
fn debug_log() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("R5_SERVO_DEBUG").is_ok_and(|v| !v.is_empty()))
}

fn lookup_protocol(raw_url: &str) -> Option<String> {
    let map = protocol_map().lock().ok()?;
    // Only the document itself counts. A host fallback would report the
    // protocol of whatever image or script came from that host last.
    map.get(raw_url).cloned()

}

const R5_EV_LOAD_STARTED: i32 = 1;
const R5_EV_LOAD_COMMITTED: i32 = 2;
const R5_EV_LOAD_FINISHED: i32 = 3;
const R5_EV_LOAD_FAILED: i32 = 4;
const R5_EV_NAVIGATION: i32 = 5;
const R5_EV_TITLE: i32 = 6;
const R5_EV_URL: i32 = 7;
const R5_EV_HOVER: i32 = 8;
const R5_EV_PROTOCOL: i32 = 9;
const R5_EV_PROGRESS: i32 = 10;
const R5_EV_CURSOR: i32 = 11;
const R5_EV_FULLSCREEN: i32 = 12;
const R5_EV_NEW_WINDOW: i32 = 13;
const R5_EV_CLOSE: i32 = 14;
const R5_EV_FAVICON: i32 = 15;
const R5_EV_DIALOG: i32 = 16;

const OK: i32 = 0;
const NO_INIT: i32 = -1;
const BAD_VALUE: i32 = -2;
const NOT_SUPPORTED: i32 = -3;

#[repr(C)]
pub struct R5ServoEvent {
    pub kind: i32,
    pub url: *const libc_char,
    pub text: *const libc_char,
    pub progress: f32,
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub current: i64,
    pub expected: i64,
    pub favicon: *const u8,
    pub favicon_width: i32,
    pub favicon_height: i32,
    pub favicon_format: i32,
}

// Avoid a libc crate: char is i8 on some targets and u8 on Haiku. The C
// side only reads the pointer as const char*.
#[allow(non_camel_case_types)]
type libc_char = u8;

type Callback = extern "C" fn(*mut std::ffi::c_void, *const R5ServoEvent);

struct Ev {
    kind: i32,
    url: String,
    text: String,
    progress: f32,
    a: i32,
    b: i32,
    c: i32,
    favicon: Vec<u8>,
    favicon_width: i32,
    favicon_height: i32,
    favicon_format: i32,
    // The event's `current`: for R5_EV_NEW_WINDOW the engine Servo made for
    // the page; for R5_EV_DIALOG the id to answer with.
    current: i64,
}

impl Ev {
    fn new(kind: i32) -> Ev {
        Ev {
            kind,
            url: String::new(),
            text: String::new(),
            progress: 0.0,
            a: 0,
            b: 0,
            c: 0,
            favicon: Vec::new(),
            favicon_width: 0,
            favicon_height: 0,
            favicon_format: 0,
            current: 0,
        }
    }
}

struct Notes {
    history: Vec<String>,
    index: usize,
    protocol: String,
    saw_red: bool,
    spins: u32,
    last_url: String,
    last_status: String,
    // Servo said a new frame is ready, so the next spin paints and reads it.
    frame_ready: bool,
    // Diagnostics: the first frame Servo announced and the first one read.
    seen_ready: bool,
    seen_frame: bool,
    last_frame_url: String,
    last_ink: usize,
    // Buttons held down, so a release reports the right one.
    buttons: u32,
}

struct Inner {
    servo: Rc<servo::Servo>,
    webview: Option<WebView>,
    context: Option<Rc<WindowRenderingContext>>,
    frame: Vec<u8>,
    width: i32,
    height: i32,
    gl: String,
    https: bool,
    // The size the context's GL storage was made with.
    storage_w: i32,
    storage_h: i32,
    // The size of `frame`.
    frame_w: i32,
    frame_h: i32,
    // Hidden by r5_servo_set_visible(). The tabs of one window share one
    // context (one Servo painter); a hidden tab must not read it back, the
    // framebuffer then holds the visible tab.
    hidden: bool,
}

// A page control waiting for the user's answer (r5_servo_control_answer()).
// Dropping one gives the page its default answer.
enum Pending {
    Dialog(SimpleDialog),
    Select(servo::SelectElement),
    Auth(AuthenticationRequest),
    File(servo::FilePicker),
    Menu(servo::ContextMenu),
}

struct Engine {
    pending: RefCell<Vec<(i64, Pending)>>,
    inner: RefCell<Inner>,
    events: Mutex<Vec<Ev>>,
    notes: Mutex<Notes>,
    // None for an engine Servo made for a page until Aomi takes it over.
    callback: std::cell::Cell<Option<Callback>>,
    callback_ctx: std::cell::Cell<*mut std::ffi::c_void>,
}

struct Bridge {
    engine: AtomicPtr<Engine>,
    id: std::sync::atomic::AtomicU64,
}

// Null once the engine is destroyed: Servo may still call the delegate of a
// WebView it has not let go of yet.
fn engine_ptr(bridge: &Bridge) -> *mut Engine {
    let engine = bridge.engine.load(Ordering::Relaxed);
    if alive_id(engine, bridge.id.load(Ordering::Relaxed)) {
        engine
    } else {
        std::ptr::null_mut()
    }
}

// The R5 clipboard (BClipboard "system"), implemented in BServoView.cpp.
// Servo's default keeps the text inside this process only on Haiku.
extern "C" {
    fn r5_servo_clipboard_set(text: *const std::ffi::c_char);
    fn r5_servo_clipboard_get() -> *mut std::ffi::c_char;
    fn r5_servo_clipboard_free(text: *mut std::ffi::c_char);
}

struct R5Clipboard;

impl servo::ClipboardDelegate for R5Clipboard {
    fn clear(&self, _webview: WebView) {
        unsafe { r5_servo_clipboard_set(std::ptr::null()) };
    }

    fn get_text(&self, _webview: WebView, request: servo::StringRequest) {
        let raw = unsafe { r5_servo_clipboard_get() };
        if raw.is_null() {
            request.failure("the clipboard holds no text".to_string());
            return;
        }
        let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned();
        unsafe { r5_servo_clipboard_free(raw) };
        request.success(text);
    }

    fn set_text(&self, _webview: WebView, new_contents: String) {
        if let Ok(text) = CString::new(new_contents.replace('\0', "")) {
            unsafe { r5_servo_clipboard_set(text.as_ptr()) };
        }
    }
}

fn emit(engine: *mut Engine, event: Ev) {
    if !alive(engine) {
        return;
    }
    let engine = unsafe { &*engine };
    if let Ok(mut events) = engine.events.lock() {
        events.push(event);
    }
}

fn flush(engine: *mut Engine) {
    if !alive(engine) {
        return;
    }
    let (callback, ctx) = {
        let engine = unsafe { &*engine };
        (engine.callback.get(), engine.callback_ctx.get())
    };
    // No callback yet: the events wait for r5_servo_set_callback().
    let Some(callback) = callback else {
        return;
    };
    let pending = {
        let engine = unsafe { &*engine };
        match engine.events.lock() {
            Ok(mut events) => std::mem::take(&mut *events),
            Err(_) => return,
        }
    };
    for event in pending {
        let url = CString::new(event.url.replace('\0', "")).unwrap_or_default();
        let text = CString::new(event.text.replace('\0', "")).unwrap_or_default();
        let c_event = R5ServoEvent {
            kind: event.kind,
            url: url.as_ptr() as *const u8,
            text: text.as_ptr() as *const u8,
            progress: event.progress,
            a: event.a,
            b: event.b,
            c: event.c,
            current: event.current,

            expected: 0,
            favicon: event.favicon.as_ptr(),
            favicon_width: event.favicon_width,
            favicon_height: event.favicon_height,
            favicon_format: event.favicon_format,
        };
        callback(ctx, &c_event);
    }
}

impl WebViewDelegate for Bridge {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        let engine = engine_ptr(self);
        if alive(engine) {
            if let Ok(mut notes) = unsafe { (*engine).notes.lock() } {
                notes.frame_ready = true;
                if !notes.seen_ready {
                    notes.seen_ready = true;
                    if debug_log() {
                        println!("r5-servo first-ready {:p}", engine);
                    }
                }
            }
        }
    }

    fn notify_url_changed(&self, webview: WebView, url: url::Url) {
        let mut event = Ev::new(R5_EV_URL);
        event.url = shown_url(&url);
        let engine = engine_ptr(self);
        if !engine.is_null() {
            if let Ok(mut inner) = unsafe { (*engine).inner.try_borrow_mut() } {
                inner.https = event.url.starts_with("https:");
            }
            let _ = webview;
        }
        emit(engine, event);
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        let mut event = Ev::new(R5_EV_TITLE);
        event.text = title.unwrap_or_default();
        emit(engine_ptr(self), event);
    }

    fn notify_status_text_changed(&self, _webview: WebView, status: Option<String>) {
        let mut event = Ev::new(R5_EV_HOVER);
        event.url = shown_text(&status.unwrap_or_default());
        emit(engine_ptr(self), event);
    }

    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        let engine = engine_ptr(self);
        let url = webview.url().map(|item| shown_url(&item)).unwrap_or_default();
        let (kind, progress, loading) = match status {
            LoadStatus::Started => (R5_EV_LOAD_STARTED, 0.05, 1),
            LoadStatus::HeadParsed => (R5_EV_PROGRESS, 0.45, 1),
            LoadStatus::Complete => (R5_EV_LOAD_FINISHED, 1.0, 0),
            LoadStatus::Stopped => (R5_EV_LOAD_FINISHED, 1.0, 0),
        };
        let mut event = Ev::new(kind);
        event.url = url.clone();
        event.progress = progress;
        emit(engine, event);
        if let Some(proto) = lookup_protocol(&url) {
            if !engine.is_null() {
                let mut changed = false;
                if let Ok(mut notes) = unsafe { (*engine).notes.lock() } {
                    if notes.protocol != proto {
                        notes.protocol = proto.clone();
                        changed = true;
                    }
                }
                if changed {
                    let mut pev = Ev::new(R5_EV_PROTOCOL);
                    pev.text = proto;
                    emit(engine, pev);
                }
            }
        }
        if !matches!(status, LoadStatus::HeadParsed) {
            let mut nav = Ev::new(R5_EV_NAVIGATION);
            nav.a = webview.can_go_back() as i32;
            nav.b = webview.can_go_forward() as i32;
            nav.c = loading;
            emit(engine, nav);
        }
    }

    fn notify_cursor_changed(&self, _webview: WebView, cursor: Cursor) {
        let mut event = Ev::new(R5_EV_CURSOR);
        event.a = cursor as u8 as i32;
        emit(engine_ptr(self), event);
    }

    fn notify_history_changed(&self, webview: WebView, entries: Vec<url::Url>, current: usize) {
        let engine = engine_ptr(self);
        if !engine.is_null() {
            if let Ok(mut notes) = unsafe { (*engine).notes.lock() } {
                notes.history = entries.iter().map(shown_url).collect();
                notes.index = current;
            }
        }
        let mut nav = Ev::new(R5_EV_NAVIGATION);
        nav.a = webview.can_go_back() as i32;
        nav.b = webview.can_go_forward() as i32;
        nav.c = !matches!(webview.load_status(), LoadStatus::Complete | LoadStatus::Stopped) as i32;
        emit(engine, nav);
    }

    fn notify_closed(&self, _webview: WebView) {
        emit(engine_ptr(self), Ev::new(R5_EV_CLOSE));
    }

    fn notify_crashed(&self, _webview: WebView, reason: String, backtrace: Option<String>) {
        // Servo already shows its crash page with a Reload button in the
        // tab (servo-script, NetworkError::Crash). The tab stops loading;
        // the reason and backtrace go to the log for a bug report.
        println!("r5-servo crashed {reason}");
        if let Some(trace) = backtrace {
            println!("r5-servo crash-backtrace {trace}");
        }
        let mut event = Ev::new(R5_EV_LOAD_FAILED);
        event.text = format!("The page crashed: {reason}");
        emit(engine_ptr(self), event);
    }

    fn hide_embedder_control(&self, _webview: WebView, control_id: servo::EmbedderControlId) {
        // The page closed a select list, file chooser or context menu
        // itself (it navigated, or removed the element). Its question is
        // dropped, so a late answer from the still open menu does nothing.
        let engine = engine_ptr(self);
        if engine.is_null() {
            return;
        }
        let removed = {
            let Ok(mut pending) = (unsafe { (*engine).pending.try_borrow_mut() }) else {
                return;
            };
            let index = pending.iter().position(|(_, item)| match item {
                Pending::Select(select) => select.id() == control_id,
                Pending::File(picker) => picker.id() == control_id,
                Pending::Menu(menu) => menu.id() == control_id,
                _ => false,
            });
            index.map(|index| pending.remove(index))
        };
        // Dropped outside the borrow: dropping answers the page.
        drop(removed);
    }

    fn notify_fullscreen_state_changed(&self, _webview: WebView, is_fullscreen: bool) {
        let mut event = Ev::new(R5_EV_FULLSCREEN);
        event.a = is_fullscreen as i32;
        emit(engine_ptr(self), event);
    }

    fn notify_favicon_changed(&self, webview: WebView) {
        let Some(image) = webview.favicon() else {
            return;
        };
        let format = match image.format {
            servo::PixelFormat::RGBA8 => 1,
            servo::PixelFormat::BGRA8 => 2,
            _ => 0,
        };
        if format == 0 {
            return;
        }
        let mut event = Ev::new(R5_EV_FAVICON);
        event.favicon = image.data().to_vec();
        event.favicon_width = image.width as i32;
        event.favicon_height = image.height as i32;
        event.favicon_format = format;
        emit(engine_ptr(self), event);
    }

    fn request_create_new(&self, _parent: WebView, request: CreateNewWebViewRequest) {
        // A page opened a window (target=_blank, window.open()). Servo gives
        // a builder, not an address: build the new WebView now, as an
        // engine without a view, and tell the parent tab. Its view adopts
        // the engine (r5_servo_set_callback). Dropping the request refuses.
        let parent = engine_ptr(self);
        if parent.is_null() {
            return;
        }
        let (width, height) = match unsafe { (*parent).inner.try_borrow() } {
            Ok(inner) => (inner.width, inner.height),
            Err(_) => return,
        };
        let Some(servo_engine) = shared_servo() else {
            return;
        };
        // The new page becomes a tab in the opener's window: same context.
        let Some((context, storage_w, storage_h, gl)) =
            context_of(parent, width, height).or_else(|| make_context(width, height))
        else {
            return;
        };
        let child = make_engine(
            servo_engine,
            context,
            (storage_w, storage_h),
            gl,
            (width, height),
            move |bridge, context| {
                request
                    .builder(context as Rc<dyn RenderingContext>)
                    .delegate(bridge)
                    .clipboard_delegate(Rc::new(R5Clipboard))
                    .build()
            },
            None,
            std::ptr::null_mut(),
        );
        // Hidden until a view adopts it (r5_servo_set_visible()): it draws
        // with the opener's context and must not appear in the opener's tab.
        if let Ok(mut inner) = unsafe { (*child).inner.try_borrow_mut() } {
            inner.hidden = true;
            if let Some(wv) = inner.webview.as_ref() {
                wv.hide();
            }
        }
        let mut event = Ev::new(R5_EV_NEW_WINDOW);
        event.a = 1;
        event.current = child as usize as i64;
        emit(parent, event);
    }

    fn request_permission(&self, _webview: WebView, request: PermissionRequest) {
        let mut event = Ev::new(R5_EV_DIALOG);
        event.a = 7;
        event.text = format!("{:?}", request.feature());
        emit(engine_ptr(self), event);
        request.deny();
    }

    fn request_authentication(&self, _webview: WebView, request: AuthenticationRequest) {
        // A site asks for a user name and password (HTTP authentication).
        // text: the address, and "proxy" on a second line for a proxy.
        let engine = engine_ptr(self);
        if engine.is_null() {
            return;
        }
        let mut event = Ev::new(R5_EV_DIALOG);
        event.a = 5;
        event.text = shown_url(&request.url());
        if request.for_proxy() {
            event.text.push_str("\nproxy");
        }
        event.current = set_pending(engine, Pending::Auth(request));
        emit(engine, event);
    }

    fn show_embedder_control(&self, _webview: WebView, control: EmbedderControl) {
        let engine = engine_ptr(self);
        if engine.is_null() {
            return;
        }
        match control {
            EmbedderControl::SimpleDialog(dialog) => {
                // The page waits until r5_servo_control_answer().
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = match &dialog {
                    SimpleDialog::Alert(_) => 1,
                    SimpleDialog::Confirm(_) => 2,
                    SimpleDialog::Prompt(_) => 3,
                };
                event.text = dialog.message().to_string();
                if let SimpleDialog::Prompt(prompt) = &dialog {
                    // "message<US>default value": U+001F (unit separator)
                    // cannot be part of either.
                    event.text.retain(|c| c != '\u{1f}');
                    event.text.push('\u{1f}');
                    event.text.push_str(&prompt.current_value().replace('\u{1f}', ""));
                }
                event.current = set_pending(engine, Pending::Dialog(dialog));

                emit(engine, event);
            }
            EmbedderControl::SelectElement(select) => {
                // One line per entry: "o<TAB>id<TAB>flags<TAB>label" for an
                // option (flags: s selected, d disabled) and "g<TAB><TAB><TAB>label"
                // for a group heading. b and c are the lower left corner of
                // the list's box in view pixels.
                let selected = select.selected_options();
                let option = |o: &servo::SelectElementOption| {
                    let mut flags = String::new();
                    if selected.contains(&o.id) {
                        flags.push('s');
                    }
                    if o.is_disabled {
                        flags.push('d');
                    }
                    format!("o\t{}\t{}\t{}", o.id, flags, o.label.replace(['\t', '\n'], " "))
                };
                let mut lines = Vec::new();
                for entry in select.options() {
                    match entry {
                        servo::SelectElementOptionOrOptgroup::Option(o) => lines.push(option(o)),
                        servo::SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                            lines.push(format!("g\t\t\t{}", label.replace(['\t', '\n'], " ")));
                            for o in options {
                                lines.push(option(o));
                            }
                        }
                    }
                }
                let position = select.position();
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = 9;
                event.b = position.min.x;
                event.c = position.max.y;
                event.text = lines.join("\n");
                event.current = set_pending(engine, Pending::Select(select));
                emit(engine, event);
            }
            EmbedderControl::FilePicker(picker) => {
                // An upload field. text: "m" (several files) or "s", a tab,
                // and the allowed extensions separated by commas.
                let patterns: Vec<String> =
                    picker.filter_patterns().iter().map(|p| p.0.clone()).collect();
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = 6;
                event.text = format!(
                    "{}\t{}",
                    if picker.allow_select_multiple() { "m" } else { "s" },
                    patterns.join(",")
                );
                event.current = set_pending(engine, Pending::File(picker));
                emit(engine, event);
            }
            EmbedderControl::ContextMenu(menu) => {
                // One line per item: "i<TAB>index<TAB>enabled(1/0)<TAB>label",
                // or "-" for a separator. Answer with the item's index.
                let mut lines = Vec::new();
                for (index, item) in menu.items().iter().enumerate() {
                    match item {
                        servo::ContextMenuItem::Item { label, enabled, .. } => lines.push(format!(
                            "i\t{}\t{}\t{}",
                            index,
                            *enabled as i32,
                            label.replace(['\t', '\n'], " ")
                        )),
                        servo::ContextMenuItem::Separator => lines.push("-".to_string()),
                    }
                }
                let position = menu.position();
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = 8;
                event.b = position.min.x;
                event.c = position.min.y;
                event.text = lines.join("\n");
                event.current = set_pending(engine, Pending::Menu(menu));
                emit(engine, event);
            }
            other => drop(other),
        }
    }
}

// Keeps a page control until it is answered, and returns its id (the
// R5_EV_DIALOG event's `current`). Several can wait at once: each answer
// names its own.
fn set_pending(engine: *mut Engine, control: Pending) -> i64 {
    if !alive(engine) {
        return 0;
    }
    static NEXT_CONTROL_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
    let id = NEXT_CONTROL_ID.fetch_add(1, Ordering::Relaxed);
    unsafe { (*engine).pending.borrow_mut().push((id, control)) };
    id
}

/// The answer to page control `id` (from R5_EV_DIALOG). accept = 0 cancels.
/// For a select list, value is the chosen option's id. For a prompt, text is
/// the entered text; for a sign-in "user\npassword"; for a file upload one
/// path per line.
#[no_mangle]
pub extern "C" fn r5_servo_control_answer(
    engine: *mut Engine,
    id: i64,
    accept: i32,
    value: i32,
    text: *const u8,
) -> i32 {
    if !alive(engine) {
        return NO_INIT;
    }
    let control = unsafe {
        let mut pending = (*engine).pending.borrow_mut();
        let index = pending.iter().position(|(pid, _)| *pid == id);
        index.map(|index| pending.remove(index).1)
    };
    let Some(control) = control else {
        return BAD_VALUE;
    };
    match control {
        Pending::Dialog(dialog) => {
            if accept == 0 {
                dialog.dismiss();
                return OK;
            }
            match dialog {
                SimpleDialog::Prompt(mut prompt) => {
                    if !text.is_null() {
                        let raw = unsafe { std::ffi::CStr::from_ptr(text as *const std::ffi::c_char) };
                        prompt.set_current_value(&raw.to_string_lossy());
                    }
                    prompt.confirm();
                }
                other => other.confirm(),
            }
        }
        Pending::Select(mut select) => {
            if accept != 0 && value >= 0 {
                let chosen = value as usize;
                if select.allow_select_multiple() {
                    // A multiple list: the chosen option is switched on or
                    // off; the others keep their state.
                    let mut selected = select.selected_options();
                    if let Some(index) = selected.iter().position(|id| *id == chosen) {
                        selected.remove(index);
                    } else {
                        selected.push(chosen);
                    }
                    select.select(selected);
                } else {
                    select.select(vec![chosen]);
                }
            }
            select.submit();
        }
        Pending::Auth(request) => {
            // text: user name, a newline, password. Dropping the request
            // cancels the sign-in.
            if accept != 0 && !text.is_null() {
                let raw = unsafe { std::ffi::CStr::from_ptr(text as *const std::ffi::c_char) };
                let raw = raw.to_string_lossy();
                let (user, password) = raw.split_once('\n').unwrap_or((&raw, ""));
                request.authenticate(user.to_string(), password.to_string());
            }
        }
        Pending::File(mut picker) => {
            // text: one path per line.
            if accept != 0 && !text.is_null() {
                let raw = unsafe { std::ffi::CStr::from_ptr(text as *const std::ffi::c_char) };
                let paths: Vec<PathBuf> = raw
                    .to_string_lossy()
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from)
                    .collect();
                picker.select(&paths);
                picker.submit();
            } else {
                picker.dismiss();
            }
        }
        Pending::Menu(menu) => {
            let action = if accept != 0 && value >= 0 {
                match menu.items().get(value as usize) {
                    Some(servo::ContextMenuItem::Item { action, enabled: true, .. }) => {
                        Some(*action)
                    }
                    _ => None,
                }
            } else {
                None
            };
            match action {
                Some(action) => menu.select(action),
                None => menu.dismiss(),
            }
        }

    }
    OK
}

fn write_buf(text: &str, buffer: *mut u8, length: i32) -> i32 {

    if buffer.is_null() || length < 2 {
        return BAD_VALUE;
    }
    let bytes = text.as_bytes();
    // Never cut a UTF-8 sequence in half.
    let mut count = bytes.len().min(length as usize - 1);
    while count > 0 && count < bytes.len() && !text.is_char_boundary(count) {
        count -= 1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, count);
        *buffer.add(count) = 0;
    }
    OK
}

fn initial_document(document: *const u8, is_html: i32) -> Option<url::Url> {
    if document.is_null() {
        return None;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(document as *const std::ffi::c_char) };
    let Ok(text) = text.to_str() else {
        println!("r5-servo initial-bad");
        return None;
    };
    let raw = if is_html != 0 {
        data_url(text)
    } else {
        text.to_string()
    };
    match url::Url::parse(&raw) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            println!("r5-servo initial-bad {error} {}", shown_text(&raw));
            None
        }
    }
}

fn data_url(html: &str) -> String {
    let mut out = String::from("data:text/html;charset=utf-8,");
    for byte in html.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn load(engine: *mut Engine, raw: &str) -> i32 {
    let Ok(parsed) = url::Url::parse(raw) else {
        println!("r5-servo load-bad {}", shown_text(raw));
        return BAD_VALUE;
    };
    println!("r5-servo load {}", shown_text(raw));
    {
        let engine = unsafe { &*engine };
        let Ok(mut inner) = engine.inner.try_borrow_mut() else {
            return NO_INIT;
        };
        inner.https = raw.starts_with("https:");
        if let Some(wv) = inner.webview.as_ref() {
            wv.load(parsed);
        }
    }
    flush(engine);
    OK
}

// A rendering context for a tab: a closed tab's one when its storage is
// large enough, otherwise a new one made at surface_size() (see there).
fn make_context(width: i32, height: i32) -> Option<(Rc<WindowRenderingContext>, i32, i32, String)> {
    static GL_PATH: Once = Once::new();
    GL_PATH.call_once(choose_gl_path);
    let size = dpi::PhysicalSize {
        width: width as u32,
        height: height as u32,
    };
    let window = raw_window_handle::HaikuWindowHandle::new(core::ptr::NonNull::dangling());
    let window_handle = unsafe {
        raw_window_handle::WindowHandle::borrow_raw(raw_window_handle::RawWindowHandle::Haiku(
            window,
        ))
    };
    let (storage_w, storage_h) = surface_size(width, height);
    let pooled = CONTEXT_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        let index = pool
            .iter()
            .position(|(_, w, h)| *w >= storage_w && *h >= storage_h);
        index.map(|index| pool.remove(index))
    });
    let (context, storage_w, storage_h) = match pooled {
        Some(entry) => entry,
        None => match WindowRenderingContext::new(
            raw_window_handle::DisplayHandle::haiku(),
            window_handle,
            dpi::PhysicalSize {
                width: storage_w as u32,
                height: storage_h as u32,
            },
        ) {
            Ok(context) => (Rc::new(context), storage_w, storage_h),
            Err(error) => {
                println!("r5-servo context-fail {error:?}");
                return None;
            }
        },
    };
    // The recorded size is the view's; the storage stays as made.
    context.resize(size);
    if let Err(error) = context.make_current() {
        println!("r5-servo current-fail {error:?}");
        CONTEXT_POOL.with(|pool| pool.borrow_mut().push((context, storage_w, storage_h)));
        return None;
    }
    let gl = format!(
        "gl vendor {} renderer {}",
        context.gleam_gl_api().get_string(0x1F00),
        context.gleam_gl_api().get_string(0x1F01)
    );
    println!("r5-servo context-ok");
    println!("r5-servo {gl}");
    Some((context, storage_w, storage_h, gl))
}

// The context of a live engine, for a new tab in the same window. Servo
// keeps one painter (one WebRender, its worker threads and its compiled
// shaders) per rendering context (servo-paint paint.rs,
// register_rendering_context, Rc::ptr_eq). A tab with a context of its own
// therefore compiled every shader again, which takes minutes on Lavapipe;
// servoshell also gives all tabs of a window one context. The painter draws
// only the WebViews that are not hidden (painter.rs,
// send_root_pipeline_display_list), so the hidden tabs stay out of the
// visible tab's frame.
fn context_of(
    sibling: *mut Engine,
    width: i32,
    height: i32,
) -> Option<(Rc<WindowRenderingContext>, i32, i32, String)> {
    if !alive(sibling) {
        return None;
    }
    let inner = unsafe { (*sibling).inner.try_borrow() }.ok()?;
    let context = inner.context.as_ref()?;
    if width > inner.storage_w || height > inner.storage_h {
        return None;
    }
    Some((
        Rc::clone(context),
        inner.storage_w,
        inner.storage_h,
        inner.gl.clone(),
    ))
}

// Whether a live engine draws with this context; None when one cannot be
// checked right now.
fn context_in_use(context: &Rc<WindowRenderingContext>) -> Option<bool> {
    let engines: Vec<usize> = ENGINES.with(|list| list.borrow().iter().map(|e| e.0).collect());
    for engine in engines {
        let engine = engine as *mut Engine;
        let Ok(inner) = (unsafe { (*engine).inner.try_borrow() }) else {
            return None;
        };
        if inner
            .context
            .as_ref()
            .is_some_and(|other| Rc::ptr_eq(other, context))
        {
            return Some(true);
        }
    }
    Some(false)
}

// The one Servo of this thread, built on first use.
fn shared_servo() -> Option<Rc<servo::Servo>> {
    if SHUT_DOWN.with(|flag| flag.get()) {
        println!("r5-servo create-after-shutdown");
        return None;
    }
    install_protocol_logger();
    Some(SHARED_SERVO.with(|slot| {
        if let Some(existing) = slot.borrow().as_ref() {
            return Rc::clone(existing);
        }
        // With a config directory Servo keeps cookies, HSTS and logins on
        // disk. A normal window passes its profile directory; a private
        // window passes none, so nothing it sees is written anywhere.
        let mut opts = Opts::default();
        if let Ok(dir) = std::env::var("AOMI_PROFILE_DIR") {
            if !dir.is_empty() {
                opts.config_dir = Some(PathBuf::from(dir));
            }
        }
        let waker = Waker {
            target: wake_target(),
        };
        let mut preferences = Preferences::default();
        preferences.network_http_proxy_uri = String::new();
        preferences.network_https_proxy_uri = String::new();
        let built = Rc::new(
            ServoBuilder::default()
                .opts(opts)
                .preferences(preferences)
                .event_loop_waker(Box::new(waker))
                .build(),
        );
        *slot.borrow_mut() = Some(Rc::clone(&built));
        built
    }))
}

// Wraps a WebView in an Engine. `make_webview` gets the delegate and the
// context and builds the WebView (a new one, or one for a page's
// window.open()). Without a callback the engine keeps its events until
// r5_servo_set_callback().
#[allow(clippy::too_many_arguments)]
fn make_engine(
    servo_engine: Rc<servo::Servo>,
    context: Rc<WindowRenderingContext>,
    storage: (i32, i32),
    gl: String,
    size: (i32, i32),
    make_webview: impl FnOnce(Rc<Bridge>, Rc<WindowRenderingContext>) -> WebView,
    callback: Option<Callback>,
    callback_ctx: *mut std::ffi::c_void,
) -> *mut Engine {
    let bridge = Rc::new(Bridge {
        engine: AtomicPtr::new(std::ptr::null_mut()),
        id: std::sync::atomic::AtomicU64::new(0),
    });
    let webview = make_webview(bridge.clone(), context.clone());
    // A new WebView starts focused; the view gives it focus when clicked.
    webview.set_focused(false);
    let engine = Box::into_raw(Box::new(Engine {
        pending: RefCell::new(Vec::new()),

        inner: RefCell::new(Inner {
            servo: servo_engine,
            webview: Some(webview),
            context: Some(context),
            frame: Vec::new(),
            width: size.0,
            height: size.1,
            gl,
            https: false,
            storage_w: storage.0,
            storage_h: storage.1,
            frame_w: 0,
            frame_h: 0,
            hidden: false,
        }),
        events: Mutex::new(Vec::new()),
        notes: Mutex::new(Notes {
            history: Vec::new(),
            index: 0,
            protocol: String::new(),
            saw_red: false,
            spins: 0,
            last_url: String::new(),
            last_status: String::new(),
            frame_ready: true,
            seen_ready: false,
            seen_frame: false,
            last_frame_url: String::new(),
            last_ink: usize::MAX,
            buttons: 0,
        }),
        callback: std::cell::Cell::new(callback),
        callback_ctx: std::cell::Cell::new(callback_ctx),
    }));
    let id = NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed);
    bridge.id.store(id, Ordering::Relaxed);
    bridge.engine.store(engine, Ordering::Relaxed);
    ENGINES.with(|list| list.borrow_mut().push((engine as usize, id)));
    engine
}

#[no_mangle]
pub extern "C" fn r5_servo_create(
    width: i32,
    height: i32,
    document: *const u8,
    document_is_html: i32,
    callback: Callback,
    callback_ctx: *mut std::ffi::c_void,
) -> *mut Engine {
    create_engine(
        std::ptr::null_mut(),
        width,
        height,
        document,
        document_is_html,
        callback,
        callback_ctx,
    )
}

/// r5_servo_create() for another tab of the window `sibling` is in: the new
/// tab draws with the sibling's context, so Servo reuses that window's
/// painter and its compiled shaders. Only one tab per window may be visible
/// (r5_servo_set_visible()). A dead or null sibling makes a context of its
/// own, as r5_servo_create().
#[no_mangle]
pub extern "C" fn r5_servo_create_sharing(
    sibling: *mut Engine,
    width: i32,
    height: i32,
    document: *const u8,
    document_is_html: i32,
    callback: Callback,
    callback_ctx: *mut std::ffi::c_void,
) -> *mut Engine {
    create_engine(
        sibling,
        width,
        height,
        document,
        document_is_html,
        callback,
        callback_ctx,
    )
}

fn create_engine(
    sibling: *mut Engine,
    width: i32,
    height: i32,
    document: *const u8,
    document_is_html: i32,
    callback: Callback,
    callback_ctx: *mut std::ffi::c_void,
) -> *mut Engine {
    let width = width.max(16);
    let height = height.max(16);
    let Some(servo_engine) = shared_servo() else {
        return std::ptr::null_mut();
    };
    let shared = context_of(sibling, width, height);
    let is_shared = shared.is_some();
    let Some((context, storage_w, storage_h, gl)) =
        shared.or_else(|| make_context(width, height))
    else {
        return std::ptr::null_mut();
    };
    let initial = initial_document(document, document_is_html);
    let servo_for_builder = Rc::clone(&servo_engine);
    let engine = make_engine(
        servo_engine,
        context,
        (storage_w, storage_h),
        gl,
        (width, height),
        move |bridge, context| {
            let mut builder =
                WebViewBuilder::new(&servo_for_builder, context as Rc<dyn RenderingContext>)
                    .delegate(bridge)
                    .clipboard_delegate(Rc::new(R5Clipboard));
            if let Some(url) = initial {
                println!("r5-servo initial {}", shown_url(&url));
                builder = builder.url(url);
            }
            builder.build()
        },
        Some(callback),
        callback_ctx,
    );
    if debug_log() {
        println!("r5-servo engine {:p} {width}x{height} shared {is_shared}", engine);
    }
    engine
}

/// Gives an engine made by Servo for a page (R5_EV_NEW_WINDOW, the engine in
/// the event's `current` field) its callback; its queued events follow.
#[no_mangle]
pub extern "C" fn r5_servo_set_callback(
    engine: *mut Engine,
    callback: Callback,
    callback_ctx: *mut std::ffi::c_void,
) -> i32 {
    if !alive(engine) {
        return NO_INIT;
    }
    unsafe {
        (*engine).callback.set(Some(callback));
        (*engine).callback_ctx.set(callback_ctx);
    }
    flush(engine);
    OK
}

/// Where Servo's waker sends its wake-up. Call it on the window thread
/// before the first r5_servo_create(); a later call changes the target.
#[no_mangle]
pub extern "C" fn r5_servo_set_waker(function: Option<WakeFn>, cookie: *mut std::ffi::c_void) {
    if let Ok(mut slot) = wake_target().lock() {
        *slot = function.map(|f| (f, cookie as usize));
    }
}

/// Runs Servo's event loop once for every tab on this thread and delivers
/// their events. Returns how many tabs have a new frame waiting.
#[no_mangle]
pub extern "C" fn r5_servo_pump() -> i32 {
    let servo = SHARED_SERVO.with(|slot| slot.borrow().clone());
    let Some(servo) = servo else {
        return 0;
    };
    servo.spin_event_loop();
    let engines: Vec<usize> = ENGINES.with(|list| list.borrow().iter().map(|e| e.0).collect());
    let mut ready = 0;
    for raw in engines {
        let engine = raw as *mut Engine;
        flush(engine);
        if alive(engine) {
            if let Ok(notes) = unsafe { (*engine).notes.lock() } {
                if notes.frame_ready {
                    ready += 1;
                }
            }
        }
    }
    ready
}

fn webview_point(x: f32, y: f32) -> WebViewPoint {
    DevicePoint::new(x, y).into()
}

fn send_input(engine: *mut Engine, event: InputEvent) -> i32 {
    if !alive(engine) {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            wv.notify_input_event(event);
            return OK;
        }
    }
    NO_INIT
}

/// The pointer moved to (x, y) in the view, in pixels.
#[no_mangle]
pub extern "C" fn r5_servo_mouse_move(engine: *mut Engine, x: f32, y: f32) -> i32 {
    send_input(engine, InputEvent::MouseMove(MouseMoveEvent::new(webview_point(x, y))))
}

/// Haiku's buttons bit field (primary 1, secondary 2, tertiary 4) after the
/// change. Down and up are worked out from the previous call.
#[no_mangle]
pub extern "C" fn r5_servo_mouse_buttons(engine: *mut Engine, x: f32, y: f32, buttons: u32) -> i32 {
    if !alive(engine) {
        return NO_INIT;
    }
    let previous = match unsafe { (*engine).notes.lock() } {
        Ok(mut notes) => {
            let previous = notes.buttons;
            notes.buttons = buttons;
            previous
        }
        Err(_) => return NO_INIT,
    };
    let map = [
        (1u32, MouseButton::Primary),
        (2u32, MouseButton::Secondary),
        (4u32, MouseButton::Auxiliary),
    ];
    let mut result = OK;
    for (bit, button) in map {
        let was = previous & bit != 0;
        let now = buttons & bit != 0;
        if was == now {
            continue;
        }
        let action = if now {
            MouseButtonAction::Down
        } else {
            MouseButtonAction::Up
        };
        result = send_input(
            engine,
            InputEvent::MouseButton(MouseButtonEvent::new(action, button, webview_point(x, y))),
        );
    }
    result
}

/// The wheel turned at (x, y). dx and dy are Haiku's wheel deltas (one notch
/// is 1.0, positive is down or right); pixels_per_notch scales them.
#[no_mangle]
pub extern "C" fn r5_servo_wheel(
    engine: *mut Engine,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    pixels_per_notch: f32,
) -> i32 {
    let delta = WheelDelta {
        // Haiku: positive is down or right. Servo: positive scrolls the view
        // up or left (embedder_traits input_events.rs, WheelDelta).
        x: (-dx * pixels_per_notch) as f64,
        y: (-dy * pixels_per_notch) as f64,
        z: 0.0,
        mode: WheelMode::DeltaPixel,
    };
    send_input(engine, InputEvent::Wheel(WheelEvent::new(delta, webview_point(x, y))))
}

// Matches R5_SERVO_KEY_* in r5_servo_abi.h.
fn named_key(code: i32) -> Option<NamedKey> {
    Some(match code {
        1 => NamedKey::Enter,
        2 => NamedKey::Backspace,
        3 => NamedKey::Tab,
        4 => NamedKey::Escape,
        5 => NamedKey::Delete,
        6 => NamedKey::ArrowLeft,
        7 => NamedKey::ArrowRight,
        8 => NamedKey::ArrowUp,
        9 => NamedKey::ArrowDown,
        10 => NamedKey::Home,
        11 => NamedKey::End,
        12 => NamedKey::PageUp,
        13 => NamedKey::PageDown,
        14 => NamedKey::Insert,
        21 => NamedKey::F1,
        22 => NamedKey::F2,
        23 => NamedKey::F3,
        24 => NamedKey::F4,
        25 => NamedKey::F5,
        26 => NamedKey::F6,
        27 => NamedKey::F7,
        28 => NamedKey::F8,
        29 => NamedKey::F9,
        30 => NamedKey::F10,
        31 => NamedKey::F11,
        32 => NamedKey::F12,
        _ => return None,
    })
}

/// A key went down (down = 1) or up. A named key (R5_SERVO_KEY_*) wins over
/// text; otherwise text is the UTF-8 the key produced. Modifiers:
/// 1 shift, 2 control, 4 alt, 8 meta.
#[no_mangle]
pub extern "C" fn r5_servo_key(
    engine: *mut Engine,
    down: i32,
    named: i32,
    text: *const u8,
    modifiers: u32,
    repeat: i32,
) -> i32 {
    let key = if let Some(name) = named_key(named) {
        Key::Named(name)
    } else {
        if text.is_null() {
            return BAD_VALUE;
        }
        let raw = unsafe { std::ffi::CStr::from_ptr(text as *const std::ffi::c_char) };
        let Ok(raw) = raw.to_str() else {
            return BAD_VALUE;
        };
        if raw.is_empty() {
            return BAD_VALUE;
        }
        Key::Character(raw.to_string())
    };
    let mut mods = Modifiers::empty();
    if modifiers & 1 != 0 {
        mods |= Modifiers::SHIFT;
    }
    if modifiers & 2 != 0 {
        mods |= Modifiers::CONTROL;
    }
    if modifiers & 4 != 0 {
        mods |= Modifiers::ALT;
    }
    if modifiers & 8 != 0 {
        mods |= Modifiers::META;
    }
    let mut event = KeyboardEvent::from_state_and_key(
        if down != 0 { KeyState::Down } else { KeyState::Up },
        key,
    );
    event.event.modifiers = mods;
    event.event.repeat = repeat != 0;
    send_input(engine, InputEvent::Keyboard(event))
}


#[no_mangle]
pub extern "C" fn r5_servo_resize(engine: *mut Engine, width: i32, height: i32) {
    if engine.is_null() {
        return;
    }
    let width = width.max(16);
    let height = height.max(16);
    if let Ok(mut inner) = unsafe { (*engine).inner.try_borrow_mut() } {
        // Never larger than the storage (see surface_size()).
        let width = width.min(inner.storage_w);
        let height = height.min(inner.storage_h);
        let size = dpi::PhysicalSize {
            width: width as u32,
            height: height as u32,
        };
        inner.width = width;
        inner.height = height;
        // Only the WebView: Servo's painter resizes the context itself and
        // returns early when the context already has the new size
        // (servo-paint painter.rs, resize_rendering_context), which left the
        // page laid out at its old size.

        if let Some(wv) = inner.webview.as_ref() {
            wv.resize(size);
        }
    }
}

#[no_mangle]
pub extern "C" fn r5_servo_load_url(engine: *mut Engine, url: *const u8) -> i32 {
    if engine.is_null() || url.is_null() {
        return BAD_VALUE;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(url as *const std::ffi::c_char) };
    let Ok(text) = text.to_str() else {
        return BAD_VALUE;
    };
    load(engine, text)
}

#[no_mangle]
pub extern "C" fn r5_servo_load_html(engine: *mut Engine, html: *const u8) -> i32 {
    if engine.is_null() || html.is_null() {
        return BAD_VALUE;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(html as *const std::ffi::c_char) };
    let Ok(text) = text.to_str() else {
        return BAD_VALUE;
    };
    load(engine, &data_url(text))
}

#[no_mangle]
pub extern "C" fn r5_servo_reload(engine: *mut Engine, bypass_cache: i32) -> i32 {
    if bypass_cache != 0 {
        return NOT_SUPPORTED;
    }
    if engine.is_null() {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            wv.reload();
        }
    }
    flush(engine);
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_stop(engine: *mut Engine) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            wv.evaluate_javascript("window.stop()", |_| {});
        }
    }
    flush(engine);
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_back(engine: *mut Engine) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            let _ = wv.go_back(1);
        }
    }
    flush(engine);
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_forward(engine: *mut Engine) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            let _ = wv.go_forward(1);
        }
    }
    flush(engine);
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_zoom(engine: *mut Engine, factor: f32, text_only: i32) -> i32 {
    if text_only != 0 {
        return NOT_SUPPORTED;
    }
    if engine.is_null() {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            wv.set_page_zoom(factor);
        }
    }
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_focus(engine: *mut Engine, focused: i32) -> i32 {
    if !alive(engine) {
        return NO_INIT;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            wv.set_focused(focused != 0);
        }
    }
    OK
}

/// Copy (0), cut (1) or paste (2) in the page, as the Edit menu or the
/// shortcut asked. Select all is the key Control+A (r5_servo_key()).
#[no_mangle]
pub extern "C" fn r5_servo_editing_action(engine: *mut Engine, action: i32) -> i32 {
    let action = match action {
        0 => servo::ClipboardAction::Copy,
        1 => servo::ClipboardAction::Cut,
        2 => servo::ClipboardAction::Paste,
        _ => return BAD_VALUE,
    };
    send_input(engine, InputEvent::EditingAction(action))
}

/// A tab that is not shown: Servo stops its animation frames for it.

#[no_mangle]
pub extern "C" fn r5_servo_set_visible(engine: *mut Engine, visible: i32) -> i32 {
    if !alive(engine) {
        return NO_INIT;
    }
    if debug_log() {
        println!("r5-servo visible {:p} {visible}", engine);
    }
    if let Ok(mut inner) = unsafe { (*engine).inner.try_borrow_mut() } {
        inner.hidden = visible == 0;
        if let Some(wv) = inner.webview.as_ref() {
            if visible != 0 {
                wv.show();
                // The tabs of a window share the context; this tab's size
                // is the one to draw at (the painter returns early when the
                // context already has it).
                wv.resize(dpi::PhysicalSize {
                    width: inner.width as u32,
                    height: inner.height as u32,
                });
            } else {
                wv.hide();
            }
        }
    }
    if visible != 0 {
        // A frame Servo announced while the tab was hidden was not read.
        if let Ok(mut notes) = unsafe { (*engine).notes.lock() } {
            notes.frame_ready = true;
        }
    }
    flush(engine);
    OK
}

/// Shuts Servo down on this thread after the last tab is destroyed. Servo
/// writes cookies, HSTS and logins to the profile only while shutting down
/// (servo-net resource_thread.rs, on Exit; sent by Drop for ServoInner), and
/// that happens nowhere else. Afterwards no new tab can be made in this
/// process: Servo cannot be built twice.
#[no_mangle]
pub extern "C" fn r5_servo_shutdown() {
    if SHUT_DOWN.with(|flag| flag.replace(true)) {
        return;
    }
    // Every engine still alive first, also one Servo made for a page that no
    // view adopted: each WebView keeps Servo alive, and Servo writes the
    // profile only when the last reference goes.
    let left: Vec<usize> = ENGINES.with(|list| list.borrow().iter().map(|e| e.0).collect());
    for engine in left {
        r5_servo_destroy(engine as *mut Engine);
    }
    let servo = SHARED_SERVO.with(|slot| slot.borrow_mut().take());

    // The last Rc: dropping it runs Servo's shutdown here, on this thread.
    drop(servo);
    // Destroying a pooled context aborts in Haiku Mesa; the process is
    // ending, so the pool is let go of without destroying them.
    CONTEXT_POOL.with(|pool| {
        for entry in pool.borrow_mut().drain(..) {
            std::mem::forget(entry);
        }
    });
}


#[no_mangle]
pub extern "C" fn r5_servo_find(
    engine: *mut Engine,
    text: *const u8,
    forward: i32,
    match_case: i32,
) -> i32 {
    if engine.is_null() || text.is_null() {
        return BAD_VALUE;
    }
    let query = unsafe { std::ffi::CStr::from_ptr(text as *const std::ffi::c_char) };
    let Ok(query) = query.to_str() else {
        return BAD_VALUE;
    };
    // Servo has no window.find(). This walks the text nodes, selects the
    // next or previous match after the current selection, wrapping around,
    // and scrolls it into view. The query goes in as a JSON-style string.
    let mut escaped = String::with_capacity(query.len() + 2);
    for ch in query.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                escaped.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => escaped.push(c),
        }
    }
    let js = format!(
        r#"(function(q, cs, fwd) {{
  if (!q || !document.body) return false;
  var norm = function(s) {{ return cs ? s : s.toLowerCase(); }};
  var needle = norm(q);
  var hits = [];
  var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (var n = walker.nextNode(); n; n = walker.nextNode()) {{
    var p = n.parentNode;
    if (p && (p.nodeName == "SCRIPT" || p.nodeName == "STYLE")) continue;
    var hay = norm(n.nodeValue), i = hay.indexOf(needle);
    while (i >= 0) {{ hits.push([n, i]); i = hay.indexOf(needle, i + 1); }}
  }}
  if (hits.length == 0) return false;
  var sel = window.getSelection(), at = -1;
  if (sel && sel.rangeCount > 0) {{
    var r = sel.getRangeAt(0);
    for (var k = 0; k < hits.length; k++)
      if (hits[k][0] == r.startContainer && hits[k][1] == r.startOffset) {{ at = k; break; }}
  }}
  var next = fwd ? (at + 1) % hits.length : (at <= 0 ? hits.length - 1 : at - 1);
  var range = document.createRange();
  range.setStart(hits[next][0], hits[next][1]);
  range.setEnd(hits[next][0], hits[next][1] + q.length);
  sel.removeAllRanges();
  sel.addRange(range);
  var el = hits[next][0].parentNode;
  if (el && el.scrollIntoView) el.scrollIntoView({{block: "center"}});
  return true;
}})("{escaped}", {}, {})"#,
        if match_case != 0 { "true" } else { "false" },
        if forward != 0 { "true" } else { "false" }
    );

    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        if let Some(wv) = inner.webview.as_ref() {
            wv.evaluate_javascript(&js, |_| {});
        }
    }
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_destroy(engine: *mut Engine) {
    if engine.is_null() {
        return;
    }
    if !alive(engine) {
        return;
    }
    // From here on no event or JavaScript result reaches the C++ view.
    ENGINES.with(|list| list.borrow_mut().retain(|item| item.0 != engine as usize));
    let boxed = unsafe { Box::from_raw(engine) };
    if let Ok(mut inner) = boxed.inner.try_borrow_mut() {
        inner.webview = None;
        // Destroying a rendering context aborts in Haiku Mesa's
        // eglDestroyImageKHR. The next tab reuses this one once no tab of
        // its window draws with it any more (r5_servo_create_sharing()).
        if let Some(ctx) = inner.context.take() {
            match context_in_use(&ctx) {
                Some(false) => {
                    let entry = (ctx, inner.storage_w, inner.storage_h);
                    CONTEXT_POOL.with(|pool| pool.borrow_mut().push(entry));
                }
                // Another tab still has it: this reference goes.
                Some(true) => drop(ctx),
                // Cannot be checked: never risk the last reference.
                None => std::mem::forget(ctx),
            }
        }

        inner.frame = Vec::new();
    }
    drop(boxed);
}

#[no_mangle]
pub extern "C" fn r5_servo_spin(engine: *mut Engine) {
    if !alive(engine) {
        return;
    }
    // Run the event loop for every tab first, outside any borrow, so the
    // delegate can update this engine while Servo works.
    r5_servo_pump();
    paint_engine(engine);
}

/// Paints the tab if Servo has a new frame for it, without running the event
/// loop. Returns 1 when r5_servo_frame() now holds a new frame.
#[no_mangle]
pub extern "C" fn r5_servo_paint(engine: *mut Engine) -> i32 {
    paint_engine(engine) as i32
}

fn paint_engine(engine: *mut Engine) -> bool {
    if !alive(engine) {
        return false;
    }
    let mut painted = false;
    {
        let engine_ref = unsafe { &*engine };
        let Ok(mut inner) = engine_ref.inner.try_borrow_mut() else {
            return false;
        };
        let mut new_frame: Option<(Vec<u8>, i32, i32)> = None;

        // Paint and read back only when Servo has a new frame: a full
        // read-back per spin is what made scrolling and loading slow.
        // A hidden tab keeps its frame_ready for when it is shown: the
        // shared framebuffer holds the visible tab of its window.
        let paint_now = !inner.hidden
            && (engine_ref
                .notes
                .lock()
                .map(|mut notes| std::mem::replace(&mut notes.frame_ready, false))
                .unwrap_or(false)
                || inner.frame.is_empty());
        let rect = DeviceIntRect::from_origin_and_size(
            DeviceIntPoint::new(0, 0),
            DeviceIntSize::new(inner.width, inner.height),
        );
        {
            let (Some(wv), Some(ctx)) = (inner.webview.as_ref(), inner.context.as_ref()) else {
                return false;
            };
            // No present(): the surface is a generic one, present() only
            // returns NoWidgetAttached and logs a warning per frame
            // (surfman base/egl/surface.rs). The painter made the context
            // current; read_to_image() reads the bound framebuffer.
            let image = if paint_now {
                wv.paint();
                // read_to_image() reads with whatever GL context is current
                // (servo-paint-api rendering_context.rs; it does not make
                // its own current). Painter::render() makes its context
                // current first, so this only guards the read against
                // another context made current in between.
                if ctx.make_current().is_ok() {
                    ctx.read_to_image(rect)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(image) = image {
                let (w, h) = (image.width() as i32, image.height() as i32);
                let bytes = image.into_raw();
                if bytes.len() >= 4 {
                    let pixel = [bytes[0], bytes[1], bytes[2], bytes[3]];
                    let mut notes = engine_ref.notes.lock().ok();
                    if let Some(notes) = notes.as_mut() {
                        if !notes.saw_red && pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40 {
                            notes.saw_red = true;
                            println!(
                                "r5-servo pixel {} {} {} {}",
                                pixel[0], pixel[1], pixel[2], pixel[3]
                            );
                        }
                    }
                    if let Some(notes) = notes.as_mut() {
                        if !notes.seen_frame {
                            notes.seen_frame = true;
                            if debug_log() {
                                println!("r5-servo first-frame {w}x{h} {:p}", engine);
                            }
                        }
                    }
                    new_frame = Some((bytes, w, h));
                }
            }
            let status = format!("{:?}", wv.load_status());
            let page = wv.url().map(|item| shown_url(&item)).unwrap_or_default();
            // Diagnostics: the first frame read for each new address.
            if let Some((bytes, _, _)) = new_frame.as_ref().filter(|_| debug_log()) {
                if let Ok(mut notes) = engine_ref.notes.lock() {
                    let new_page = notes.last_frame_url != page;
                    {
                        // Sampled pixels that are not white: 0 is an empty page.
                        let ink = bytes
                            .chunks_exact(4)
                            .step_by(97)
                            .filter(|px| px[0] < 200 || px[1] < 200 || px[2] < 200)
                            .count();
                        if new_page || ink != notes.last_ink {
                            println!("r5-servo frame-for {:p} ink {ink} {page}", engine);
                        }
                        notes.last_frame_url = page.clone();
                        notes.last_ink = ink;
                    }
                }
            }
            if let Some(proto) = lookup_protocol(&page) {
                let mut changed = false;
                if let Ok(mut notes) = engine_ref.notes.lock() {
                    if notes.protocol != proto {
                        notes.protocol = proto.clone();
                        changed = true;
                    }
                }
                if changed {
                    let mut pev = Ev::new(R5_EV_PROTOCOL);
                    pev.text = proto;
                    emit(engine, pev);
                }
            }
            if let Ok(mut notes) = engine_ref.notes.lock() {
                notes.spins += 1;
                let changed = notes.last_status != status || notes.last_url != page;
                if changed {
                    println!("r5-servo beat {} {status} {page}", notes.spins);
                    notes.last_status = status;
                    notes.last_url = page;
                }
            }
        }
        if let Some((bytes, w, h)) = new_frame {
            // The frame keeps its own size: a resize between two frames must
            // not make r5_servo_frame() report a size the buffer lacks.
            inner.frame = bytes;
            inner.frame_w = w;
            inner.frame_h = h;
            painted = true;
        }
    }
    flush(engine);
    painted
}


#[no_mangle]
pub extern "C" fn r5_servo_frame(
    engine: *mut Engine,
    rgba: *mut *const u8,
    width: *mut i32,
    height: *mut i32,
) -> i32 {
    if engine.is_null() || rgba.is_null() || width.is_null() || height.is_null() {
        return BAD_VALUE;
    }
    let Ok(inner) = (unsafe { (*engine).inner.try_borrow() }) else {
        return NO_INIT;
    };
    if inner.frame.is_empty() {
        return NO_INIT;
    }
    unsafe {
        *rgba = inner.frame.as_ptr();
        *width = inner.frame_w;
        *height = inner.frame_h;
    }
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_url(engine: *mut Engine, buffer: *mut u8, length: i32) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    let Ok(inner) = (unsafe { (*engine).inner.try_borrow() }) else {
        return NO_INIT;
    };
    let text = inner
        .webview
        .as_ref()
        .and_then(|wv| wv.url())
        .map(|item| shown_url(&item))
        .unwrap_or_default();
    write_buf(&text, buffer, length)
}

#[no_mangle]
pub extern "C" fn r5_servo_title(engine: *mut Engine, buffer: *mut u8, length: i32) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    let Ok(inner) = (unsafe { (*engine).inner.try_borrow() }) else {
        return NO_INIT;
    };
    let text = inner
        .webview
        .as_ref()
        .and_then(|wv| wv.page_title())
        .unwrap_or_default();
    write_buf(&text, buffer, length)
}

#[no_mangle]
pub extern "C" fn r5_servo_protocol(engine: *mut Engine, buffer: *mut u8, length: i32) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    let Ok(notes) = (unsafe { (*engine).notes.lock() }) else {
        return NO_INIT;
    };
    if notes.protocol.is_empty() {
        return NOT_SUPPORTED;
    }
    write_buf(&notes.protocol, buffer, length)
}

#[no_mangle]
pub extern "C" fn r5_servo_history(
    engine: *mut Engine,
    forward: i32,
    index: i32,
    buffer: *mut u8,
    length: i32,
) -> i32 {
    if engine.is_null() || index < 0 {
        return BAD_VALUE;
    }
    let Ok(notes) = (unsafe { (*engine).notes.lock() }) else {
        return NO_INIT;
    };
    let items: Vec<&String> = if forward == 0 {
        notes.history.iter().take(notes.index).collect()
    } else {
        notes.history.iter().skip(notes.index + 1).collect()
    };
    let Some(item) = items.get(index as usize) else {
        return BAD_VALUE;
    };
    write_buf(item, buffer, length)
}

#[no_mangle]
pub extern "C" fn r5_servo_gl(engine: *mut Engine, buffer: *mut u8, length: i32) -> i32 {
    if engine.is_null() {
        return NO_INIT;
    }
    let Ok(inner) = (unsafe { (*engine).inner.try_borrow() }) else {
        return NO_INIT;
    };
    write_buf(&inner.gl, buffer, length)
}
