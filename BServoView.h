/*
 * SPDX-FileCopyrightText: 2026 R5OS Contributors
 * SPDX-License-Identifier: MIT
 *
 * Servo's page view for any program: link libr5servo.so. The calls and the
 * message codes match the numbers in distro/apps/aomi/WebContentView.h.
 * What Servo 0.8.0 cannot do returns B_NOT_SUPPORTED. R5Book: servo-view.html.
 *
 * The r5servo package installs:
 *   header  -> /boot/system/develop/headers/r5/BServoView.h
 *   library -> /boot/system/lib/libr5servo.so
 *
 * Servo runs one engine per process, on the thread that first creates a
 * view: keep every BServoView of a program in one window. The view takes
 * mouse, wheel and keys, and draws the page at its own size.

 */
#ifndef R5_BSERVO_VIEW_H
#define R5_BSERVO_VIEW_H


#include <Messenger.h>
#include <String.h>
#include <View.h>


class BWebSession {
public:
								BWebSession(bool privateSession);
			bool				IsPrivate() const { return fPrivate; }

private:
			bool				fPrivate;
			uint32				_reserved[4];
};


// Same codes as distro/apps/aomi/WebContentView.h. Delete on the swap.
enum {
	AOMI_LOAD_STARTED			= 'aLdS',
	AOMI_LOAD_COMMITTED			= 'aLdC',
	AOMI_LOAD_FINISHED			= 'aLdF',
	AOMI_LOAD_FAILED			= 'aLdX',
	AOMI_NAVIGATION				= 'aNav',
	AOMI_TITLE_CHANGED			= 'aTit',
	AOMI_URL_CHANGED			= 'aUrl',
	AOMI_FAVICON				= 'aFav',
	AOMI_HOVER_LINK				= 'aHov',
	AOMI_SECURITY				= 'aSec',
	AOMI_PROTOCOL				= 'aPrt',
	AOMI_PROGRESS				= 'aPrg',
	AOMI_FOCUS_WANTED			= 'aFoc',
	AOMI_CURSOR					= 'aCur',
	AOMI_FULLSCREEN				= 'aFsc',
	AOMI_NEW_WINDOW				= 'aNww',
	AOMI_CLOSE_WANTED			= 'aClw',
	AOMI_DOWNLOAD_ADDED			= 'aDla',
	AOMI_DOWNLOAD_PROGRESS		= 'aDlp',
	AOMI_DOWNLOAD_FINISHED		= 'aDlf',
	AOMI_DOWNLOAD_FAILED		= 'aDlx',
	AOMI_DIALOG					= 'aDlg'
};


enum {
	AOMI_SECURITY_UNKNOWN		= 0,
	AOMI_SECURITY_SECURE		= 1,
	AOMI_SECURITY_NOT_SECURE	= 2,
	AOMI_SECURITY_CERTIFICATE	= 3
};


enum {
	AOMI_DIALOG_ALERT			= 1,
	AOMI_DIALOG_CONFIRM			= 2,
	AOMI_DIALOG_PROMPT			= 3,
	AOMI_DIALOG_LEAVE			= 4,
	AOMI_DIALOG_AUTH			= 5,
	AOMI_DIALOG_FILE			= 6,
	AOMI_DIALOG_PERMISSION		= 7,
	AOMI_DIALOG_CONTEXT			= 8
};


class BServoView : public BView {
public:
								BServoView(const char* name, BWebSession* session);
	virtual						~BServoView();

			void				SetListener(const BMessenger& listener);
			BWebSession*		Session() const { return fSession; }

	virtual	status_t			LoadURL(const char* url);
	virtual	status_t			LoadHTML(const char* html, const char* baseURL);
	virtual	status_t			Reload(bool bypassCache);
	virtual	status_t			StopLoading();

	virtual	status_t			GoBack();
	virtual	status_t			GoForward();
	virtual	status_t			GetBackList(BMessage* items);
	virtual	status_t			GetForwardList(BMessage* items);

	virtual	status_t			URL(BString* url) const;
	virtual	status_t			Title(BString* title) const;
	virtual	status_t			SetZoom(float factor, bool textOnly);
	virtual	status_t			Find(const char* text, bool forward, bool matchCase);
	virtual	status_t			SavePage(const char* path);
	virtual	status_t			ViewSource();
	virtual	status_t			Print();
	virtual	status_t			Protocol(BString* protocol) const;

	virtual	status_t			Cancel(int32 id);
	virtual	status_t			Open(int32 id);
	virtual	status_t			ShowInTracker(int32 id);

	virtual	void				AttachedToWindow();
	virtual	void				DetachedFromWindow();
	virtual	void				Draw(BRect updateRect);
	virtual	void				MouseDown(BPoint where);
	virtual	void				MouseUp(BPoint where);
	virtual	void				MouseMoved(BPoint where, uint32 transit,
									const BMessage* dragMessage);
	virtual	void				KeyDown(const char* bytes, int32 numBytes);
	virtual	void				KeyUp(const char* bytes, int32 numBytes);

	virtual	void				FrameResized(float width, float height);
	virtual	void				MessageReceived(BMessage* message);
	virtual	void				MakeFocus(bool focused = true);
	virtual	void				Show();
	virtual	void				Hide();

private:
			status_t			_Start(const char* document, bool documentIsHTML);
			status_t			_FillHistory(BMessage* items, bool forward) const;
			status_t			_Report(uint32 what, const char* url, const char* text,
									float progress, int32 a, int32 b, int32 c);
			void				_OnEvent(const struct r5_servo_event* event);
			void				_CopyFrame();
	static	void				_Callback(void* context, const struct r5_servo_event* event);

			BMessenger			fListener;
			BWebSession*		fSession;
			bool				fPrivate;
			struct r5_servo*	fEngine;
			class BBitmap*		fBitmap;
			class BMessageRunner* fRunner;
};


#endif
