/*
 * SPDX-FileCopyrightText: 2026 R5OS Contributors
 * SPDX-License-Identifier: MIT
 *
 * One R5 window linked to libr5servo.so.
 * R5_SERVO_PROOF=1 loads a red data document, then https://http2.pro/api/v1.
 */


#include "BServoView.h"

#include <Application.h>
#include <Window.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>


static const char* kRedHtml =
	"<html style='background:#ff0000;margin:0;height:100%'>"
	"<body style='background:#ff0000;margin:0;height:100%'>R5</body></html>";


static const char*
code_name(uint32 what)
{
	switch (what) {
		case AOMI_LOAD_STARTED: return "load-started";
		case AOMI_LOAD_COMMITTED: return "load-committed";
		case AOMI_LOAD_FINISHED: return "load-finished";
		case AOMI_LOAD_FAILED: return "load-failed";
		case AOMI_NAVIGATION: return "navigation";
		case AOMI_TITLE_CHANGED: return "title";
		case AOMI_URL_CHANGED: return "url";
		case AOMI_FAVICON: return "favicon";
		case AOMI_HOVER_LINK: return "hover";
		case AOMI_SECURITY: return "security";
		case AOMI_PROTOCOL: return "protocol";
		case AOMI_PROGRESS: return "progress";
		case AOMI_CURSOR: return "cursor";
		case AOMI_FULLSCREEN: return "fullscreen";
		case AOMI_NEW_WINDOW: return "new-window";
		case AOMI_CLOSE_WANTED: return "close";
		case AOMI_DIALOG: return "dialog";
		default: return "other";
	}
}


class TestWindow : public BWindow {
public:
								TestWindow();
	virtual	void				MessageReceived(BMessage* message);
	virtual	bool				QuitRequested();
			void				Open(const char* urlOrNull);

private:
			BWebSession*		fSession;
			BServoView*			fView;
			bool				fProof;
			bool				fHttp2;
};


class TestApp : public BApplication {
public:
								TestApp();
	virtual	void				ArgvReceived(int32 argc, char** argv);
	virtual	void				ReadyToRun();

private:
			TestWindow*			fWindow;
			const char*			fUrl;
};


TestWindow::TestWindow()
	:
	BWindow(BRect(80, 60, 879, 559), "R5 Servo", B_TITLED_WINDOW,
		B_NOT_RESIZABLE | B_NOT_ZOOMABLE),
	fProof(getenv("R5_SERVO_PROOF") != NULL),
	fHttp2(false)
{
	fSession = new BWebSession(false);
	fView = new BServoView("page", fSession);
	AddChild(fView);
	fView->SetListener(BMessenger(this));
}


void
TestWindow::Open(const char* urlOrNull)
{
	if (fProof || urlOrNull == NULL)
		fView->LoadHTML(kRedHtml, "aomi:newtab");
	else
		fView->LoadURL(urlOrNull);
}


void
TestWindow::MessageReceived(BMessage* message)
{
	const char* url = "";
	const char* text = "";
	const char* protocol = "";
	message->FindString("url", &url);
	message->FindString("text", &text);
	message->FindString("protocol", &protocol);
	float progress = -1;
	message->FindFloat("progress", &progress);
	printf("r5-servo msg %s url=%s text=%s protocol=%s progress=%.2f\n",
		code_name(message->what), url != NULL ? url : "",
		text != NULL ? text : "", protocol != NULL ? protocol : "", progress);
	fflush(stdout);
	sync();

	if (fProof && !fHttp2 && message->what == AOMI_LOAD_FINISHED && url != NULL
		&& strncmp(url, "data:", 5) == 0) {
		fHttp2 = true;
		printf("r5-servo load-http2 https://http2.pro/api/v1\n");
		fflush(stdout);
		sync();
		fView->LoadURL("https://http2.pro/api/v1");
	}
	BWindow::MessageReceived(message);
}


bool
TestWindow::QuitRequested()
{
	be_app->PostMessage(B_QUIT_REQUESTED);
	return true;
}


TestApp::TestApp()
	:
	BApplication("application/x-vnd.R5-servo-test"),
	fWindow(NULL),
	fUrl(NULL)
{
}


void
TestApp::ArgvReceived(int32 argc, char** argv)
{
	if (argc > 1)
		fUrl = argv[1];
}


void
TestApp::ReadyToRun()
{
	fWindow = new TestWindow();
	fWindow->Show();
	fWindow->Open(fUrl);
}


int
main()
{
	TestApp app;
	app.Run();
	return 0;
}
