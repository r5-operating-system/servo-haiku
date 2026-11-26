// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 R5OS Contributors
//
// Servo itself is MPL-2.0. This file is the R5 wrapper.
// Event numbers match distro/servo/r5_servo_abi.h.

use std::cell::RefCell;
use std::ffi::CString;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, Ordering};

use servo::{
    AuthenticationRequest, CreateNewWebViewRequest, Cursor, DeviceIntPoint, DeviceIntRect,
    DeviceIntSize, EmbedderControl, FilePicker, JSValue, LoadStatus, PermissionRequest,
    Preferences, RenderingContext, ServoBuilder, SimpleDialog, WebView, WebViewBuilder,
    WebViewDelegate, WindowRenderingContext,
};

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
const R5_EV_PAGE_TEXT: i32 = 17;

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
        }
    }
}

struct Notes {
    history: Vec<String>,
    index: usize,
    protocol: String,
    asked_body: bool,
    saw_red: bool,
    spins: u32,
    last_url: String,
    last_status: String,
}

struct Inner {
    servo: servo::Servo,
    webview: WebView,
    context: Rc<WindowRenderingContext>,
    frame: Vec<u8>,
    width: i32,
    height: i32,
    gl: String,
    https: bool,
}

struct Engine {
    inner: RefCell<Inner>,
    events: Mutex<Vec<Ev>>,
    notes: Mutex<Notes>,
    callback: Callback,
    callback_ctx: *mut std::ffi::c_void,
}

struct Bridge {
    engine: AtomicPtr<Engine>,
}

fn engine_ptr(bridge: &Bridge) -> *mut Engine {
    bridge.engine.load(Ordering::Relaxed)
}

fn emit(engine: *mut Engine, event: Ev) {
    if engine.is_null() {
        return;
    }
    let engine = unsafe { &*engine };
    if let Ok(mut events) = engine.events.lock() {
        events.push(event);
    }
}

fn flush(engine: *mut Engine) {
    if engine.is_null() {
        return;
    }
    let pending = {
        let engine = unsafe { &*engine };
        match engine.events.lock() {
            Ok(mut events) => std::mem::take(&mut *events),
            Err(_) => return,
        }
    };
    let (callback, ctx) = {
        let engine = unsafe { &*engine };
        (engine.callback, engine.callback_ctx)
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
            current: 0,
            expected: 0,
            favicon: event.favicon.as_ptr(),
            favicon_width: event.favicon_width,
            favicon_height: event.favicon_height,
            favicon_format: event.favicon_format,
        };
        callback(ctx, &c_event);
    }
}

fn protocol_from_body(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("\"http2\":1")
        || lower.contains("\"http2\": 1")
        || lower.contains("\"http2\":true")
        || lower.contains("http/2")
    {
        return Some("h2");
    }
    if lower.contains("\"http2\":0")
        || lower.contains("\"http2\": 0")
        || lower.contains("\"http2\":false")
        || lower.contains("http/1.1")
    {
        return Some("http/1.1");
    }
    None
}

impl WebViewDelegate for Bridge {
    fn notify_url_changed(&self, webview: WebView, url: url::Url) {
        let mut event = Ev::new(R5_EV_URL);
        event.url = url.to_string();
        let engine = engine_ptr(self);
        if !engine.is_null() {
            if let Ok(mut inner) = unsafe { (*engine).inner.try_borrow_mut() } {
                inner.https = event.url.starts_with("https:");
            }
            if let Ok(mut notes) = unsafe { (*engine).notes.lock() } {
                notes.asked_body = false;
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
        event.url = status.unwrap_or_default();
        emit(engine_ptr(self), event);
    }

    fn notify_load_status_changed(&self, webview: WebView, status: LoadStatus) {
        let engine = engine_ptr(self);
        let url = webview.url().map(|item| item.to_string()).unwrap_or_default();
        let (kind, progress, loading) = match status {
            LoadStatus::Started => (R5_EV_LOAD_STARTED, 0.05, 1),
            LoadStatus::HeadParsed => (R5_EV_PROGRESS, 0.45, 1),
            LoadStatus::Complete => (R5_EV_LOAD_FINISHED, 1.0, 0),
            LoadStatus::Stopped => (R5_EV_LOAD_FINISHED, 1.0, 0),
        };
        let mut event = Ev::new(kind);
        event.url = url;
        event.progress = progress;
        emit(engine, event);
        if !matches!(status, LoadStatus::HeadParsed) {
            let mut nav = Ev::new(R5_EV_NAVIGATION);
            nav.a = webview.can_go_back() as i32;
            nav.b = webview.can_go_forward() as i32;
            nav.c = loading;
            emit(engine, nav);
        }
        if matches!(status, LoadStatus::Complete) && !engine.is_null() {
            if let Ok(mut notes) = unsafe { (*engine).notes.lock() } {
                notes.asked_body = false;
            }
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
                notes.history = entries.iter().map(|item| item.to_string()).collect();
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

    fn request_create_new(&self, _parent: WebView, _request: CreateNewWebViewRequest) {
        // Servo 0.8 does not expose the target URL on the request. The
        // window still hears that a new view was asked for. Dropping the
        // request leaves the new view unbuilt.
        let mut event = Ev::new(R5_EV_NEW_WINDOW);
        event.a = 1;
        emit(engine_ptr(self), event);
    }

    fn request_permission(&self, _webview: WebView, request: PermissionRequest) {
        let mut event = Ev::new(R5_EV_DIALOG);
        event.a = 7;
        event.text = format!("{:?}", request.feature());
        emit(engine_ptr(self), event);
        request.deny();
    }

    fn request_authentication(&self, _webview: WebView, request: AuthenticationRequest) {
        let mut event = Ev::new(R5_EV_DIALOG);
        event.a = 5;
        event.url = request.url().to_string();
        emit(engine_ptr(self), event);
    }

    fn show_embedder_control(&self, _webview: WebView, control: EmbedderControl) {
        let engine = engine_ptr(self);
        match control {
            EmbedderControl::SimpleDialog(dialog) => {
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = match &dialog {
                    SimpleDialog::Alert(_) => 1,
                    SimpleDialog::Confirm(_) => 2,
                    SimpleDialog::Prompt(_) => 3,
                };
                event.text = dialog.message().to_string();
                emit(engine, event);
                // The reply is not wired back. Dismiss so the page does not wait.
                dialog.dismiss();
            }
            EmbedderControl::FilePicker(picker) => {
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = 6;
                emit(engine, event);
                picker.dismiss();
            }
            EmbedderControl::ContextMenu(menu) => {
                let mut event = Ev::new(R5_EV_DIALOG);
                event.a = 8;
                emit(engine, event);
                menu.dismiss();
            }
            other => drop(other),
        }
    }
}

fn write_buf(text: &str, buffer: *mut u8, length: i32) -> i32 {
    if buffer.is_null() || length < 2 {
        return BAD_VALUE;
    }
    let bytes = text.as_bytes();
    let count = bytes.len().min(length as usize - 1);
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
            println!("r5-servo initial-bad {error} {raw}");
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
        println!("r5-servo load-bad {raw}");
        return BAD_VALUE;
    };
    println!("r5-servo load {raw}");
    {
        let engine = unsafe { &*engine };
        let Ok(mut inner) = engine.inner.try_borrow_mut() else {
            return NO_INIT;
        };
        inner.https = raw.starts_with("https:");
        inner.webview.load(parsed);
    }
    flush(engine);
    OK
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
    let width = width.max(16);
    let height = height.max(16);
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
    let context = match WindowRenderingContext::new(
        raw_window_handle::DisplayHandle::haiku(),
        window_handle,
        size,
    ) {
        Ok(context) => context,
        Err(error) => {
            println!("r5-servo context-fail {error:?}");
            return std::ptr::null_mut();
        }
    };
    if let Err(error) = context.make_current() {
        println!("r5-servo current-fail {error:?}");
        return std::ptr::null_mut();
    }
    let gl = format!(
        "gl vendor {} renderer {}",
        context.gleam_gl_api().get_string(0x1F00),
        context.gleam_gl_api().get_string(0x1F01)
    );
    println!("r5-servo context-ok");
    println!("r5-servo {gl}");

    let mut preferences = Preferences::default();
    preferences.network_http_proxy_uri = String::new();
    preferences.network_https_proxy_uri = String::new();
    let servo_engine = ServoBuilder::default().preferences(preferences).build();
    servo_engine.setup_logging();
    let initial = initial_document(document, document_is_html);
    let context = Rc::new(context);
    let bridge = Rc::new(Bridge {
        engine: AtomicPtr::new(std::ptr::null_mut()),
    });
    let mut builder = WebViewBuilder::new(
        &servo_engine,
        context.clone() as Rc<dyn RenderingContext>,
    )
    .delegate(bridge.clone());
    if let Some(url) = initial {
        println!("r5-servo initial {}", url);
        builder = builder.url(url);
    }
    let webview = builder.build();

    let engine = Box::into_raw(Box::new(Engine {
        inner: RefCell::new(Inner {
            servo: servo_engine,
            webview,
            context,
            frame: Vec::new(),
            width,
            height,
            gl,
            https: false,
        }),
        events: Mutex::new(Vec::new()),
        notes: Mutex::new(Notes {
            history: Vec::new(),
            index: 0,
            protocol: String::new(),
            asked_body: false,
            saw_red: false,
            spins: 0,
            last_url: String::new(),
            last_status: String::new(),
        }),
        callback,
        callback_ctx,
    }));
    bridge.engine.store(engine, Ordering::Relaxed);
    engine
}

#[no_mangle]
pub extern "C" fn r5_servo_resize(engine: *mut Engine, width: i32, height: i32) {
    if engine.is_null() {
        return;
    }
    let width = width.max(16);
    let height = height.max(16);
    let size = dpi::PhysicalSize {
        width: width as u32,
        height: height as u32,
    };
    if let Ok(mut inner) = unsafe { (*engine).inner.try_borrow_mut() } {
        inner.width = width;
        inner.height = height;
        inner.context.resize(size);
        inner.webview.resize(size);
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
        inner.webview.reload();
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
        inner.webview.evaluate_javascript("window.stop()", |_| {});
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
        let _ = inner.webview.go_back(1);
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
        let _ = inner.webview.go_forward(1);
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
        inner.webview.set_page_zoom(factor);
    }
    OK
}

#[no_mangle]
pub extern "C" fn r5_servo_focus(engine: *mut Engine, focused: i32) {
    if engine.is_null() {
        return;
    }
    if let Ok(inner) = unsafe { (*engine).inner.try_borrow() } {
        inner.webview.set_focused(focused != 0);
    }
}

#[no_mangle]
pub extern "C" fn r5_servo_spin(engine: *mut Engine) {
    if engine.is_null() {
        return;
    }
    let mut ask_body = false;
    {
        let engine_ref = unsafe { &*engine };
        let Ok(mut inner) = engine_ref.inner.try_borrow_mut() else {
            return;
        };
        inner.servo.spin_event_loop();
        inner.webview.paint();
        inner.context.present();
        let rect = DeviceIntRect::from_origin_and_size(
            DeviceIntPoint::new(0, 0),
            DeviceIntSize::new(inner.width, inner.height),
        );
        if let Some(image) = inner.context.read_to_image(rect) {
            let bytes = image.as_raw();
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
                inner.frame = bytes.to_vec();
            }
        }
        let status = format!("{:?}", inner.webview.load_status());
        let page = inner
            .webview
            .url()
            .map(|item| item.to_string())
            .unwrap_or_default();
        if let Ok(mut notes) = engine_ref.notes.lock() {
            notes.spins += 1;
            let changed = notes.last_status != status || notes.last_url != page;
            if changed {
                println!("r5-servo beat {} {status} {page}", notes.spins);
                notes.last_status = status;
                notes.last_url = page;
            }
        }
        if inner.https {
            if let Ok(mut notes) = engine_ref.notes.lock() {
                if !notes.asked_body
                    && matches!(inner.webview.load_status(), LoadStatus::Complete)
                {
                    notes.asked_body = true;
                    ask_body = true;
                }
            }
        }
        if ask_body {
            let engine_for_js = engine;
            inner.webview.evaluate_javascript(
                "document.body ? document.body.innerText : ''",
                move |result| {
                    let text = match result {
                        Ok(JSValue::String(text)) => text,
                        other => format!("{other:?}"),
                    };
                    let clipped: String = text.chars().take(500).collect();
                    let mut event = Ev::new(R5_EV_PAGE_TEXT);
                    event.text = clipped.clone();
                    emit(engine_for_js, event);
                    if let Some(protocol) = protocol_from_body(&clipped) {
                        if let Ok(mut notes) = unsafe { (*engine_for_js).notes.lock() } {
                            notes.protocol = protocol.to_string();
                        }
                        let mut event = Ev::new(R5_EV_PROTOCOL);
                        event.text = protocol.to_string();
                        println!("r5-servo protocol {protocol}");
                        emit(engine_for_js, event);
                    }
                },
            );
        }
    }
    flush(engine);
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
        *width = inner.width;
        *height = inner.height;
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
    let text = inner.webview.url().map(|item| item.to_string()).unwrap_or_default();
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
    let text = inner.webview.page_title().unwrap_or_default();
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
