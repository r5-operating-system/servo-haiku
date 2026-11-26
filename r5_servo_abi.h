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


}


#endif
