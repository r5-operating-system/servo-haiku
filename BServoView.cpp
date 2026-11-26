/*
 * SPDX-FileCopyrightText: 2026 R5OS Contributors
 * SPDX-License-Identifier: MIT
 */


#include "BServoView.h"

#include "r5_servo_abi.h"

#include <Bitmap.h>
#include <Message.h>
#include <MessageRunner.h>
#include <String.h>
#include <Window.h>

#include <stdio.h>
#include <string.h>


static const uint32 kSpinMessage = 'svSp';


static status_t
engine_status(int32 code)
{
	switch (code) {
		case 0:
			return B_OK;
		case -1:
			return B_NO_INIT;
		case -2:
			return B_BAD_VALUE;
		case -3:
			return B_NOT_SUPPORTED;
		default:
			return B_ERROR;
	}
}


BWebSession::BWebSession(bool privateSession)
	:
	fPrivate(privateSession)
{
	_reserved[0] = _reserved[1] = _reserved[2] = _reserved[3] = 0;
}


BServoView::BServoView(const char* name, BWebSession* session)
	:
	BView(name != NULL ? name : "servo", B_WILL_DRAW | B_FRAME_EVENTS | B_NAVIGABLE
		| B_FULL_UPDATE_ON_RESIZE),
	fSession(session),
	fPrivate(session != NULL && session->IsPrivate()),
	fEngine(NULL),
	fBitmap(NULL),
	fRunner(NULL)
{
	// The page stays opaque. Glass is the window's, not the page's.
	SetViewColor(255, 255, 255);
}


BServoView::~BServoView()
{
	delete fRunner;
	delete fBitmap;
	// fEngine is left allocated. Dropping the GL surface makes
	// eglDestroyImageKHR fail and abort the team. Round 0 leaks it.
}


void
BServoView::SetListener(const BMessenger& listener)
{
	fListener = listener;
}


void
BServoView::AttachedToWindow()
{
	BView::AttachedToWindow();
	// The engine is created on the first load, with that document as the
	// initial URL. Creating it here and then calling load() races Servo's
	// about:blank, and the real document is dropped.
	BMessage spin(kSpinMessage);
	fRunner = new BMessageRunner(BMessenger(this), &spin, 30000);
}


void
BServoView::DetachedFromWindow()
{
	delete fRunner;
	fRunner = NULL;
	BView::DetachedFromWindow();
}


status_t
BServoView::_Start(const char* document, bool documentIsHTML)
{
	if (fEngine != NULL)
		return B_OK;
	// LoadURL can arrive on the application thread. Bounds() needs the
	// window lock once the view is attached.
	BWindow* window = Window();
	bool unlock = false;
	if (window != NULL && !window->IsLocked()) {
		if (!window->Lock())
			return B_ERROR;
		unlock = true;
	}
	int32 width = (int32)Bounds().Width() + 1;
	int32 height = (int32)Bounds().Height() + 1;
	if (width < 16)
		width = 16;
	if (height < 16)
		height = 16;
	fEngine = r5_servo_create(width, height, document, documentIsHTML ? 1 : 0,
		&_Callback, this);
	status_t status = fEngine != NULL ? B_OK : B_ERROR;
	if (status == B_OK && fPrivate) {
		fprintf(stderr,
			"r5-servo private session is not isolated on disk yet\n");
	}
	if (status == B_OK) {
		char gl[512];
		if (r5_servo_gl(fEngine, gl, (int32)sizeof(gl)) == 0)
			fprintf(stderr, "r5-servo %s\n", gl);
	}
	if (unlock)
		window->Unlock();
	return status;
}


status_t
BServoView::LoadURL(const char* url)
{
	if (fEngine == NULL)
		return _Start(url, false);
	return engine_status(r5_servo_load_url(fEngine, url));
}


status_t
BServoView::LoadHTML(const char* html, const char* baseURL)
{
	(void)baseURL;
	if (html == NULL)
		return B_BAD_VALUE;
	if (fEngine == NULL)
		return _Start(html, true);
	// Servo 0.8 is given a data: document. Relative links are not resolved
	// against baseURL.
	return engine_status(r5_servo_load_html(fEngine, html));
}


status_t
BServoView::Reload(bool bypassCache)
{
	if (bypassCache)
		return B_NOT_SUPPORTED;
	if (fEngine == NULL)
		return B_NO_INIT;
	return engine_status(r5_servo_reload(fEngine, 0));
}


status_t
BServoView::StopLoading()
{
	if (fEngine == NULL)
		return B_NO_INIT;
	return engine_status(r5_servo_stop(fEngine));
}


status_t
BServoView::GoBack()
{
	if (fEngine == NULL)
		return B_NO_INIT;
	return engine_status(r5_servo_back(fEngine));
}


status_t
BServoView::GoForward()
{
	if (fEngine == NULL)
		return B_NO_INIT;
	return engine_status(r5_servo_forward(fEngine));
}


status_t
BServoView::_FillHistory(BMessage* items, bool forward) const
{
	if (items == NULL)
		return B_BAD_VALUE;
	if (fEngine == NULL)
		return B_NO_INIT;
	char url[2048];
	for (int32 index = 0; index < 64; index++) {
		int32 status = r5_servo_history(fEngine, forward ? 1 : 0, index, url,
			(int32)sizeof(url));
		if (status != 0)
			break;
		items->AddString("url", url);
		items->AddString("title", url);
	}
	return B_OK;
}


status_t
BServoView::GetBackList(BMessage* items)
{
	return _FillHistory(items, false);
}


status_t
BServoView::GetForwardList(BMessage* items)
{
	return _FillHistory(items, true);
}


status_t
BServoView::URL(BString* url) const
{
	if (url == NULL)
		return B_BAD_VALUE;
	if (fEngine == NULL)
		return B_NO_INIT;
	char buffer[4096];
	int32 status = r5_servo_url(fEngine, buffer, (int32)sizeof(buffer));
	if (status != 0)
		return engine_status(status);
	*url = buffer;
	return B_OK;
}


status_t
BServoView::Title(BString* title) const
{
	if (title == NULL)
		return B_BAD_VALUE;
	if (fEngine == NULL)
		return B_NO_INIT;
	char buffer[4096];
	int32 status = r5_servo_title(fEngine, buffer, (int32)sizeof(buffer));
	if (status != 0)
		return engine_status(status);
	*title = buffer;
	return B_OK;
}


status_t
BServoView::SetZoom(float factor, bool textOnly)
{
	if (textOnly)
		return B_NOT_SUPPORTED;
	if (fEngine == NULL)
		return B_NO_INIT;
	return engine_status(r5_servo_zoom(fEngine, factor, 0));
}


status_t
BServoView::Find(const char* text, bool forward, bool matchCase)
{
	(void)text;
	(void)forward;
	(void)matchCase;
	return B_NOT_SUPPORTED;
}


status_t
BServoView::SavePage(const char* path)
{
	(void)path;
	return B_NOT_SUPPORTED;
}


status_t
BServoView::ViewSource()
{
	return B_NOT_SUPPORTED;
}


status_t
BServoView::Print()
{
	return B_NOT_SUPPORTED;
}


status_t
BServoView::Protocol(BString* protocol) const
{
	if (protocol == NULL)
		return B_BAD_VALUE;
	if (fEngine == NULL)
		return B_NO_INIT;
	char buffer[32];
	int32 status = r5_servo_protocol(fEngine, buffer, (int32)sizeof(buffer));
	if (status != 0)
		return engine_status(status);
	*protocol = buffer;
	return B_OK;
}


status_t
BServoView::Cancel(int32 id)
{
	(void)id;
	return B_NOT_SUPPORTED;
}


status_t
BServoView::Open(int32 id)
{
	(void)id;
	return B_NOT_SUPPORTED;
}


status_t
BServoView::ShowInTracker(int32 id)
{
	(void)id;
	return B_NOT_SUPPORTED;
}


void
BServoView::Draw(BRect updateRect)
{
	(void)updateRect;
	if (fBitmap != NULL)
		DrawBitmap(fBitmap, Bounds());
}


void
BServoView::FrameResized(float width, float height)
{
	BView::FrameResized(width, height);
	if (fEngine == NULL)
		return;
	int32 w = (int32)width + 1;
	int32 h = (int32)height + 1;
	if (w < 16)
		w = 16;
	if (h < 16)
		h = 16;
	r5_servo_resize(fEngine, w, h);
}


void
BServoView::MakeFocus(bool focused)
{
	BView::MakeFocus(focused);
	if (fEngine != NULL)
		r5_servo_focus(fEngine, focused ? 1 : 0);
}


void
BServoView::MessageReceived(BMessage* message)
{
	if (message->what == kSpinMessage) {
		if (fEngine != NULL) {
			r5_servo_spin(fEngine);
			_CopyFrame();
		}
		return;
	}
	BView::MessageReceived(message);
}


void
BServoView::_CopyFrame()
{
	const uint8* rgba = NULL;
	int32 width = 0;
	int32 height = 0;
	if (r5_servo_frame(fEngine, &rgba, &width, &height) != 0 || rgba == NULL)
		return;
	if (fBitmap == NULL || fBitmap->Bounds().IntegerWidth() + 1 != width
		|| fBitmap->Bounds().IntegerHeight() + 1 != height) {
		delete fBitmap;
		fBitmap = new BBitmap(BRect(0, 0, width - 1, height - 1), B_RGBA32);
	}
	uint8* dest = (uint8*)fBitmap->Bits();
	int32 stride = fBitmap->BytesPerRow();
	for (int32 y = 0; y < height; y++) {
		const uint8* src = rgba + y * width * 4;
		uint8* row = dest + y * stride;
		for (int32 x = 0; x < width; x++) {
			row[x * 4 + 0] = src[x * 4 + 2];
			row[x * 4 + 1] = src[x * 4 + 1];
			row[x * 4 + 2] = src[x * 4 + 0];
			row[x * 4 + 3] = src[x * 4 + 3];
		}
	}
	Invalidate();
}


status_t
BServoView::_Report(uint32 what, const char* url, const char* text, float progress,
	int32 a, int32 b, int32 c)
{
	if (!fListener.IsValid())
		return B_NO_INIT;
	BMessage message(what);
	if (url != NULL && url[0] != '\0')
		message.AddString("url", url);
	if (text != NULL && text[0] != '\0')
		message.AddString("text", text);
	if (what == AOMI_PROGRESS)
		message.AddFloat("progress", progress);
	if (what == AOMI_NAVIGATION) {
		message.AddBool("canBack", a != 0);
		message.AddBool("canForward", b != 0);
		message.AddBool("loading", c != 0);
	}
	if (what == AOMI_LOAD_FAILED)
		message.AddInt32("error", a);
	if (what == AOMI_CURSOR)
		message.AddInt32("cursor", a);
	if (what == AOMI_FULLSCREEN)
		message.AddBool("on", a != 0);
	if (what == AOMI_NEW_WINDOW)
		message.AddBool("primary", a != 0);
	if (what == AOMI_SECURITY)
		message.AddInt32("state", a);
	if (what == AOMI_PROTOCOL && text != NULL)
		message.AddString("protocol", text);
	if (what == AOMI_DIALOG)
		message.AddInt32("kind", a);
	return fListener.SendMessage(&message);
}


void
BServoView::_Callback(void* context, const r5_servo_event* event)
{
	if (context == NULL || event == NULL)
		return;
	static_cast<BServoView*>(context)->_OnEvent(event);
}


void
BServoView::_OnEvent(const r5_servo_event* event)
{
	switch (event->kind) {
		case R5_EV_LOAD_STARTED:
			_Report(AOMI_LOAD_STARTED, event->url, NULL, 0, 0, 0, 0);
			_Report(AOMI_PROGRESS, NULL, NULL, 0.05f, 0, 0, 0);
			break;
		case R5_EV_LOAD_COMMITTED:
			_Report(AOMI_LOAD_COMMITTED, event->url, NULL, 0, 0, 0, 0);
			break;
		case R5_EV_LOAD_FINISHED:
			_Report(AOMI_LOAD_FINISHED, event->url, NULL, 0, 0, 0, 0);
			_Report(AOMI_PROGRESS, NULL, NULL, 1.0f, 0, 0, 0);
			break;
		case R5_EV_LOAD_FAILED:
			_Report(AOMI_LOAD_FAILED, event->url, event->text, 0, event->a, 0, 0);
			break;
		case R5_EV_NAVIGATION:
			_Report(AOMI_NAVIGATION, NULL, NULL, 0, event->a, event->b, event->c);
			break;
		case R5_EV_TITLE:
			if (fListener.IsValid()) {
				BMessage message(AOMI_TITLE_CHANGED);
				message.AddString("title", event->text != NULL ? event->text : "");
				fListener.SendMessage(&message);
			}
			break;
		case R5_EV_URL:
			_Report(AOMI_URL_CHANGED, event->url, NULL, 0, 0, 0, 0);
			if (event->url != NULL && strncmp(event->url, "https:", 6) == 0)
				_Report(AOMI_SECURITY, NULL, "https", 0, AOMI_SECURITY_SECURE, 0, 0);
			else if (event->url != NULL && strncmp(event->url, "http:", 5) == 0)
				_Report(AOMI_SECURITY, NULL, "http", 0, AOMI_SECURITY_NOT_SECURE, 0, 0);
			break;
		case R5_EV_HOVER:
			if (fListener.IsValid()) {
				BMessage message(AOMI_HOVER_LINK);
				message.AddString("url", event->url != NULL ? event->url : "");
				fListener.SendMessage(&message);
			}
			break;
		case R5_EV_PROTOCOL:
			_Report(AOMI_PROTOCOL, NULL, event->text, 0, 0, 0, 0);
			break;
		case R5_EV_PROGRESS:
			_Report(AOMI_PROGRESS, NULL, NULL, event->progress, 0, 0, 0);
			break;
		case R5_EV_CURSOR:
			_Report(AOMI_CURSOR, NULL, NULL, 0, event->a, 0, 0);
			break;
		case R5_EV_FULLSCREEN:
			_Report(AOMI_FULLSCREEN, NULL, NULL, 0, event->a, 0, 0);
			break;
		case R5_EV_NEW_WINDOW:
			_Report(AOMI_NEW_WINDOW, event->url, NULL, 0, event->a, 0, 0);
			break;
		case R5_EV_CLOSE:
			_Report(AOMI_CLOSE_WANTED, NULL, NULL, 0, 0, 0, 0);
			break;
		case R5_EV_FAVICON:
		{
			if (!fListener.IsValid() || event->favicon == NULL || event->favicon_width < 1)
				break;
			BBitmap icon(BRect(0, 0, event->favicon_width - 1, event->favicon_height - 1),
				B_RGBA32);
			uint8* dest = (uint8*)icon.Bits();
			int32 pixels = event->favicon_width * event->favicon_height;
			const uint8* src = event->favicon;
			if (event->favicon_format == 2) {
				memcpy(dest, src, (size_t)pixels * 4);
			} else if (event->favicon_format == 1) {
				for (int32 i = 0; i < pixels; i++) {
					dest[i * 4 + 0] = src[i * 4 + 2];
					dest[i * 4 + 1] = src[i * 4 + 1];
					dest[i * 4 + 2] = src[i * 4 + 0];
					dest[i * 4 + 3] = src[i * 4 + 3];
				}
			} else
				break;
			BMessage archived;
			if (icon.Archive(&archived) == B_OK) {
				BMessage message(AOMI_FAVICON);
				message.AddMessage("icon", &archived);
				fListener.SendMessage(&message);
			}
			break;
		}
		case R5_EV_DIALOG:
			_Report(AOMI_DIALOG, NULL, event->text, 0, event->a, 0, 0);
			break;
		case R5_EV_PAGE_TEXT:
			fprintf(stderr, "r5-servo page-text %s\n",
				event->text != NULL ? event->text : "");
			break;
		default:
			break;
	}
}
