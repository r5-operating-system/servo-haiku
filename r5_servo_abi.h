/*
 * SPDX-FileCopyrightText: 2026 R5OS Contributors
 * SPDX-License-Identifier: MIT
 *
 * Private ABI between libr5servo's Rust engine and BServoView.
 * The numbers are not the Aomi message codes. Keep them in sync with
 * distro/servo/engine/src/lib.rs (R5_EV_*).
 */
#ifndef R5_SERVO_ABI_H
#define R5_SERVO_ABI_H


#include <SupportDefs.h>


enum {
	R5_EV_LOAD_STARTED		= 1,
	R5_EV_LOAD_COMMITTED	= 2,
	R5_EV_LOAD_FINISHED		= 3,
	R5_EV_LOAD_FAILED		= 4,
	R5_EV_NAVIGATION		= 5,
	R5_EV_TITLE				= 6,
	R5_EV_URL				= 7,
	R5_EV_HOVER				= 8,
	R5_EV_PROTOCOL			= 9,
	R5_EV_PROGRESS			= 10,
	R5_EV_CURSOR			= 11,
	R5_EV_FULLSCREEN		= 12,
	R5_EV_NEW_WINDOW		= 13,
	R5_EV_CLOSE				= 14,
	R5_EV_FAVICON			= 15,
	R5_EV_DIALOG			= 16,
	R5_EV_PAGE_TEXT			= 17
};


struct r5_servo_event {
	int32		kind;
	const char*	url;
	const char*	text;
	float		progress;
	int32		a;
	int32		b;
	int32		c;
	int64		current;
	int64		expected;
	const uint8* favicon;
	int32		favicon_width;
	int32		favicon_height;
	int32		favicon_format;
};


struct r5_servo;
class BMessage;

typedef void (*r5_servo_callback)(void* context, const r5_servo_event* event);


extern "C" {


r5_servo*	r5_servo_create(int32 width, int32 height, const char* document,
	int32 document_is_html, r5_servo_callback callback, void* context);
void		r5_servo_resize(r5_servo* engine, int32 width, int32 height);
status_t	r5_servo_load_url(r5_servo* engine, const char* url);
status_t	r5_servo_load_html(r5_servo* engine, const char* html);
status_t	r5_servo_reload(r5_servo* engine, int32 bypass_cache);
status_t	r5_servo_stop(r5_servo* engine);
status_t	r5_servo_back(r5_servo* engine);
status_t	r5_servo_forward(r5_servo* engine);
status_t	r5_servo_zoom(r5_servo* engine, float factor, int32 text_only);
status_t	r5_servo_focus(r5_servo* engine, int32 focused);
void		r5_servo_spin(r5_servo* engine);
status_t	r5_servo_frame(r5_servo* engine, const uint8** rgba, int32* width,
	int32* height);
status_t	r5_servo_url(r5_servo* engine, char* buffer, int32 length);
status_t	r5_servo_title(r5_servo* engine, char* buffer, int32 length);
status_t	r5_servo_protocol(r5_servo* engine, char* buffer, int32 length);
status_t	r5_servo_history(r5_servo* engine, int32 forward, int32 index,
	char* buffer, int32 length);
status_t	r5_servo_gl(r5_servo* engine, char* buffer, int32 length);
status_t	r5_servo_find(r5_servo* engine, const char* text, int32 forward,
	int32 match_case);
void		r5_servo_destroy(r5_servo* engine);

// Since 0.8.0-3. Servo runs one instance per process, on the thread of the
// one browser window. The waker is called from Servo's own threads; it should
// only post a message to that window, which then calls r5_servo_pump() (or
// r5_servo_spin() for the visible tab) on the window thread.
typedef void (*r5_servo_wake_function)(void* cookie);
void		r5_servo_set_waker(r5_servo_wake_function wake, void* cookie);
int32		r5_servo_pump(void);
// Paints the tab if Servo has a new frame for it (no event loop). Returns 1
// when r5_servo_frame() now holds a new frame.
int32		r5_servo_paint(r5_servo* engine);

// Input, in view pixels. Buttons is Haiku's bit field after the change.
// Wheel deltas are Haiku's (one notch is 1.0, positive is down or right).
status_t	r5_servo_mouse_move(r5_servo* engine, float x, float y);
status_t	r5_servo_mouse_buttons(r5_servo* engine, float x, float y,
	uint32 buttons);
status_t	r5_servo_wheel(r5_servo* engine, float x, float y, float dx,
	float dy, float pixels_per_notch);

enum {
	R5_SERVO_KEY_NONE		= 0,
	R5_SERVO_KEY_ENTER		= 1,
	R5_SERVO_KEY_BACKSPACE	= 2,
	R5_SERVO_KEY_TAB		= 3,
	R5_SERVO_KEY_ESCAPE		= 4,
	R5_SERVO_KEY_DELETE		= 5,
	R5_SERVO_KEY_LEFT		= 6,
	R5_SERVO_KEY_RIGHT		= 7,
	R5_SERVO_KEY_UP			= 8,
	R5_SERVO_KEY_DOWN		= 9,
	R5_SERVO_KEY_HOME		= 10,
	R5_SERVO_KEY_END		= 11,
	R5_SERVO_KEY_PAGE_UP	= 12,
	R5_SERVO_KEY_PAGE_DOWN	= 13,
	R5_SERVO_KEY_INSERT		= 14,
	R5_SERVO_KEY_F1			= 21	// F1 to F12 are 21 to 32
};

enum {
	R5_SERVO_MOD_SHIFT		= 1,
	R5_SERVO_MOD_CONTROL	= 2,
	R5_SERVO_MOD_ALT		= 4,
	R5_SERVO_MOD_META		= 8
};

// A named key wins over text. Text is the key's UTF-8.
status_t	r5_servo_key(r5_servo* engine, int32 down, int32 named_key,
	const char* text, uint32 modifiers, int32 repeat);

// The largest view this process shows, normally the screen. Call before
// r5_servo_create(): each tab's GL storage is made this large once, and a view
// is never larger (resizing the storage would mean destroying it).
void		r5_servo_set_max_size(int32 width, int32 height);

// The answer to page control `id` (the R5_EV_DIALOG event's `current`).
// Several can wait at once; each answer names its own. accept = 0 cancels.
// value: the chosen option id of a select list. text: a prompt's text, a
// sign-in's "user\npassword", or a file upload's paths, one per line.
status_t	r5_servo_control_answer(r5_servo* engine, int64 id, int32 accept,
	int32 value, const char* text);

// A Haiku input message for the view: B_MOUSE_DOWN, B_MOUSE_UP,
// B_MOUSE_MOVED, B_MOUSE_WHEEL_CHANGED, B_KEY_DOWN, B_KEY_UP and the unmapped
// key messages. The point is "be:view_where" when the message has it,
// otherwise (x, y) in view pixels. Implemented in BServoView.cpp; both
// BServoView and Aomi use it.
status_t	r5_servo_input_message(r5_servo* engine, const BMessage* message,
	float x, float y);

// Shows page control `id` (an R5_EV_DIALOG) with r5_servo_ask_control() and
// answers it. Call on the thread of the engine; a window thread stays drawn
// while it waits. NULL labels are "OK" and "Cancel". Implemented in
// BServoView.cpp.
status_t	r5_servo_show_control(r5_servo* engine, int64 id, class BView* view,
	int32 kind, const char* text, float x, float y, const char* okLabel,
	const char* cancelLabel);

// The words of r5_servo_ask_control()'s windows, translated by the caller.
// signInTo is a format with one %s for the address.
typedef struct r5_servo_control_labels {
	const char*	ok;
	const char*	cancel;
	const char*	user;
	const char*	password;
	const char*	signIn;
	const char*	signInTo;
} r5_servo_control_labels;

// Shows the user what an R5_EV_DIALOG asks and returns the answer, without
// answering the engine (any thread; a window thread stays drawn): alert (1),
// confirm (2), prompt (3, text "message", U+001F, "default"), sign-in (5, text the
// address; answer "user\npassword"), file upload (6; answer one path per
// line), context menu (8) and select list (9; value the chosen id). (x, y)
// is in view's coordinates. Free *answer with r5_servo_free_answer(); it
// may hold a password and is cleared first. NULL labels: English.
status_t	r5_servo_ask_control(class BView* view, int32 kind,
	const char* text, float x, float y,
	const r5_servo_control_labels* labels, int32* accept, int32* value,
	char** answer);
void		r5_servo_free_answer(char* answer);

// Gives an engine Servo made for a page (R5_EV_NEW_WINDOW, the engine in the
// event's `current` field) its callback. Its waiting events follow.
status_t	r5_servo_set_callback(r5_servo* engine, r5_servo_callback callback,
	void* context);

// Copy (0), cut (1) or paste (2) in the page (B_COPY, B_CUT, B_PASTE).
// Select all is the key Control+A.
status_t	r5_servo_editing_action(r5_servo* engine, int32 action);

// A tab that is not shown: Servo stops its animation frames for it.
status_t	r5_servo_set_visible(r5_servo* engine, int32 visible);

// After the last tab of the process (on the engine's thread): every engine
// left is destroyed, then Servo shuts down and writes
// cookies, HSTS and logins to the profile. No new tab can be made afterwards.
void		r5_servo_shutdown(void);

// r5_servo_create() for another tab of the window `sibling` belongs to: the
// new tab draws with the sibling's GL context, so Servo reuses that window's
// painter and its compiled shaders instead of compiling them again. Keep one
// tab per window visible (r5_servo_set_visible()); a hidden tab is not read
// back. A null or destroyed sibling gives a context of its own.
r5_servo*	r5_servo_create_sharing(r5_servo* sibling, int32 width,
	int32 height, const char* document, int32 document_is_html,
	r5_servo_callback callback, void* context);





}


#endif
