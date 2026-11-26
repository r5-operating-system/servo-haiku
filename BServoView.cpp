/*
 * SPDX-FileCopyrightText: 2026 R5OS Contributors
 * SPDX-License-Identifier: MIT
 */


#include "BServoView.h"

#include "r5_servo_abi.h"

#include <Alert.h>
#include <Button.h>
#include <FilePanel.h>
#include <LayoutBuilder.h>
#include <Node.h>
#include <Path.h>
#include <StringView.h>
#include <TextControl.h>
#include <Bitmap.h>
#include <ByteOrder.h>
#include <Font.h>
#include <InterfaceDefs.h>
#include <Message.h>
#include <MenuItem.h>
#include <MessageRunner.h>
#include <PopUpMenu.h>
#include <Screen.h>
#include <String.h>
#include <Window.h>

#include <Clipboard.h>
#include <OS.h>

#include <map>
#include <vector>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>


static const uint32 kSpinMessage = 'svSp';
static const uint32 kControlMessage = 'svCt';
static const uint32 kStartMessage = 'svSt';

// A load asked for before the view could start its engine on its window's
// thread (Servo lives on the thread that creates it). Kept outside the class
// so BServoView's size does not change.
struct PendingStart {
	BString	document;
	bool	isHTML;
};
static std::map<const BServoView*, PendingStart> sPendingStarts;


// The R5 clipboard for Servo (lib.rs, R5Clipboard). NULL clears it.
extern "C" void
r5_servo_clipboard_set(const char* text)
{
	if (!be_clipboard->Lock())
		return;
	be_clipboard->Clear();
	if (text != NULL) {
		BMessage* data = be_clipboard->Data();
		if (data != NULL)
			data->AddData("text/plain", B_MIME_TYPE, text, strlen(text));
	}
	be_clipboard->Commit();
	be_clipboard->Unlock();
}


extern "C" char*
r5_servo_clipboard_get()
{
	if (!be_clipboard->Lock())
		return NULL;
	char* copy = NULL;
	const char* text = NULL;
	ssize_t length = 0;
	BMessage* data = be_clipboard->Data();
	if (data != NULL && data->FindData("text/plain", B_MIME_TYPE,
			(const void**)&text, &length) == B_OK && length >= 0) {
		copy = (char*)malloc(length + 1);
		if (copy != NULL) {
			memcpy(copy, text, length);
			copy[length] = '\0';
		}
	}
	be_clipboard->Unlock();
	return copy;
}


extern "C" void
r5_servo_clipboard_free(char* text)
{
	free(text);
}



// Haiku's modifiers to R5_SERVO_MOD_*. The shortcut key (Alt by default)
// acts as Control inside a page, so copy and paste in a form use the same
// keys as everywhere in R5.
static uint32
servo_modifiers(uint32 modifiers)
{
	uint32 out = 0;
	if ((modifiers & B_SHIFT_KEY) != 0)
		out |= R5_SERVO_MOD_SHIFT;
	if ((modifiers & (B_CONTROL_KEY | B_COMMAND_KEY)) != 0)
		out |= R5_SERVO_MOD_CONTROL;
	if ((modifiers & B_OPTION_KEY) != 0)
		out |= R5_SERVO_MOD_META;
	return out;
}


// Waits on sem. On a window's thread the window keeps being drawn while it
// waits, as BAlert::Go() and BPopUpMenu do.
static void
wait_keeping_window_drawn(sem_id sem)
{
	BWindow* window = dynamic_cast<BWindow*>(
		BLooper::LooperForThread(find_thread(NULL)));
	if (window != NULL) {
		status_t status;
		do {
			status = acquire_sem_etc(sem, 1, B_RELATIVE_TIMEOUT, 50000);
			window->UpdateIfNeeded();
		} while (status == B_TIMED_OUT || status == B_INTERRUPTED);
	} else {
		while (acquire_sem(sem) == B_INTERRUPTED)
			;
	}
}


// Overwrites a string that may hold a password before it is freed.
static void
wipe_string(BString& text)
{
	int32 length = text.Length();
	if (length <= 0)
		return;
	memset(text.LockBuffer(length), 0, length);
	text.UnlockBuffer(0);
}


static const uint32 kControlOK = 'svOk';
static const uint32 kControlCancel = 'svCn';


// A small modal question window: a message and one or two text fields.
class ControlWindow : public BWindow {
public:
	ControlWindow(const char* title, const char* message, const char* field1,
		const char* value1, const char* field2, bool hideSecond,
		const char* okLabel, const char* cancelLabel)
		:
		BWindow(BRect(0, 0, 360, 120), title, B_MODAL_WINDOW_LOOK,
			B_MODAL_APP_WINDOW_FEEL,
			B_NOT_ZOOMABLE | B_NOT_RESIZABLE | B_AUTO_UPDATE_SIZE_LIMITS
				| B_CLOSE_ON_ESCAPE),
		fSem(create_sem(0, "servo control")),
		fAccepted(false),
		fSecond(NULL)
	{
		BStringView* label = new BStringView("message", message);
		fFirst = new BTextControl("first", field1, value1, NULL);
		// The second field is there only for a sign-in; a hidden view takes
		// no room in the layout.
		fSecond = new BTextControl("second", field2 != NULL ? field2 : "", "",
			NULL);
		if (hideSecond)
			fSecond->TextView()->HideTyping(true);
		BButton* ok = new BButton("ok", okLabel, new BMessage(kControlOK));
		BButton* cancel = new BButton("cancel", cancelLabel,
			new BMessage(kControlCancel));
		BLayoutBuilder::Group<>(this, B_VERTICAL, B_USE_DEFAULT_SPACING)
			.SetInsets(B_USE_WINDOW_SPACING)
			.Add(label)
			.Add(fFirst)
			.Add(fSecond)
			.AddGroup(B_HORIZONTAL)
				.AddGlue()
				.Add(cancel)
				.Add(ok)
			.End();
		if (field2 == NULL)
			fSecond->Hide();
		SetDefaultButton(ok);
		fFirst->MakeFocus(true);
		CenterOnScreen();
	}

	virtual ~ControlWindow()
	{
		delete_sem(fSem);
	}

	virtual void MessageReceived(BMessage* message)
	{
		if (message->what == kControlOK || message->what == kControlCancel) {
			fAccepted = message->what == kControlOK;
			fFirstText = fFirst->Text();
			fSecondText = fSecond->Text();
			release_sem(fSem);
			return;
		}
		BWindow::MessageReceived(message);
	}

	virtual bool QuitRequested()
	{
		// Escape or the close box: cancelled.
		release_sem(fSem);
		return false;
	}

	// Shows the window, waits, and quits it. True when OK was chosen.
	bool Go(BString* first, BString* second)
	{
		sem_id sem = fSem;
		Show();
		wait_keeping_window_drawn(sem);
		bool accepted = false;
		if (Lock()) {
			accepted = fAccepted;
			if (first != NULL)
				*first = fFirstText;
			if (second != NULL)
				*second = fSecondText;
			// The second field may be a password: clear the window's copies.
			wipe_string(fFirstText);
			wipe_string(fSecondText);
			fSecond->SetText("");
			Quit();
		}
		return accepted;
	}

private:
	sem_id			fSem;
	bool			fAccepted;
	BTextControl*	fFirst;
	BTextControl*	fSecond;
	BString			fFirstText;
	BString			fSecondText;
};


// Collects what an open file panel chose. The panel sends the refs, then a
// B_CANCEL whenever it is hidden.
class PanelTarget : public BLooper {
public:
	PanelTarget()
		:
		BLooper("servo file panel"),
		fSem(create_sem(0, "servo file panel"))
	{
	}

	virtual ~PanelTarget()
	{
		delete_sem(fSem);
	}

	virtual void MessageReceived(BMessage* message)
	{
		if (message->what == B_REFS_RECEIVED) {
			entry_ref ref;
			for (int32 i = 0; message->FindRef("refs", i, &ref) == B_OK; i++) {
				BPath path(&ref);
				if (path.InitCheck() == B_OK)
					fPaths << path.Path() << "\n";
			}
			return;
		}
		if (message->what == B_CANCEL) {
			release_sem(fSem);
			return;
		}
		BLooper::MessageReceived(message);
	}

	sem_id Sem() const { return fSem; }
	BString Paths() const { return fPaths; }

private:
	sem_id	fSem;
	BString	fPaths;
};


// Shows directories and files with one of the page's extensions.
class ExtensionFilter : public BRefFilter {
public:
	ExtensionFilter(const BString& patterns)
	{
		int32 start = 0;
		while (start < patterns.Length()) {
			int32 end = patterns.FindFirst(',', start);
			if (end < 0)
				end = patterns.Length();
			BString one;
			patterns.CopyInto(one, start, end - start);
			one.Trim();
			if (one.Length() > 0)
				fExtensions.push_back(BString(".") << one.ToLower());
			start = end + 1;
		}
	}

	bool Empty() const { return fExtensions.empty(); }

	virtual bool Filter(const entry_ref* ref, BNode* node,
		struct stat_beos* stat, const char* mimeType)
	{
		(void)stat;
		(void)mimeType;
		if (node != NULL && node->IsDirectory())
			return true;
		BString name(ref->name);
		name.ToLower();
		for (size_t i = 0; i < fExtensions.size(); i++) {
			if (name.EndsWith(fExtensions[i]))
				return true;
		}
		return false;
	}

private:
	std::vector<BString>	fExtensions;
};


static BMenuItem*
run_menu(BView* view, BPopUpMenu* menu, float x, float y)
{
	if (menu->CountItems() == 0)
		return NULL;
	BPoint where(x, y);
	if (view != NULL && view->LockLooper()) {
		where = view->ConvertToScreen(where);
		view->UnlockLooper();
	}
	// Synchronous; on the window thread BPopUpMenu::_WaitMenu keeps the
	// window drawn.
	return menu->Go(where, false, true);
}


extern "C" status_t
r5_servo_ask_control(BView* view, int32 kind, const char* text, float x,
	float y, const r5_servo_control_labels* labels, int32* accept,
	int32* value, char** answer)
{
	if (accept == NULL || value == NULL || answer == NULL)
		return B_BAD_VALUE;
	*accept = 0;
	*value = -1;
	*answer = NULL;
	r5_servo_control_labels fallback = { "OK", "Cancel", "User name:",
		"Password:", "Sign in", "Sign in to %s" };
	const r5_servo_control_labels& l = labels != NULL ? *labels : fallback;
	BString all(text != NULL ? text : "");
	BString reply;
	switch (kind) {
		case 1:
		{
			BAlert* alert = new BAlert("page", all.String(), l.ok);
			alert->Go();
			*accept = 1;
			break;
		}
		case 2:
		{
			BAlert* alert = new BAlert("page", all.String(), l.cancel, l.ok,
				NULL, B_WIDTH_AS_USUAL, B_WARNING_ALERT);
			alert->SetShortcut(0, B_ESCAPE);
			*accept = alert->Go() == 1 ? 1 : 0;
			break;
		}
		case 3:
		{
			// prompt(): "message", U+001F, "default value". Either may hold
			// line breaks; the separator never appears in them.
			BString message(all);
			BString preset;
			int32 separator = all.FindFirst('\x1f');
			if (separator >= 0) {
				all.CopyInto(message, 0, separator);
				all.CopyInto(preset, separator + 1,
					all.Length() - separator - 1);
			}
			ControlWindow* window = new ControlWindow("page", message.String(),
				NULL, preset.String(), NULL, false, l.ok, l.cancel);
			*accept = window->Go(&reply, NULL) ? 1 : 0;
			break;
		}
		case 5:
		{
			// Sign-in: "address" and maybe "proxy" on a second line.
			BString address(all);
			int32 newline = all.FindFirst('\n');
			if (newline >= 0)
				all.CopyInto(address, 0, newline);
			BString message;
			message.SetToFormat(l.signInTo, address.String());
			BString user;
			BString password;
			ControlWindow* window = new ControlWindow(l.signIn,
				message.String(), l.user, "", l.password, true, l.signIn,
				l.cancel);
			*accept = window->Go(&user, &password) ? 1 : 0;
			reply << user << "\n" << password;
			wipe_string(user);
			wipe_string(password);
			break;
		}
		case 6:
		{
			// An upload field: "m" or "s", a tab, extensions by commas.
			bool multiple = all.Length() > 0 && all[0] == 'm';
			BString patterns;
			int32 tab = all.FindFirst('\t');
			if (tab >= 0)
				all.CopyInto(patterns, tab + 1, all.Length() - tab - 1);
			ExtensionFilter* filter = new ExtensionFilter(patterns);
			PanelTarget* target = new PanelTarget();
			target->Run();
			sem_id sem = target->Sem();
			BMessenger messenger(target);
			BFilePanel* panel = new BFilePanel(B_OPEN_PANEL, &messenger, NULL,
				B_FILE_NODE, multiple, NULL, filter->Empty() ? NULL : filter,
				true, true);
			panel->Show();
			wait_keeping_window_drawn(sem);
			delete panel;
			delete filter;
			if (target->Lock()) {
				reply = target->Paths();
				target->Quit();
			}
			*accept = reply.Length() > 0 ? 1 : 0;
			break;
		}
		case 8:
		case 9:
		{
			// 8: context menu, "i<TAB>index<TAB>enabled<TAB>label" or "-".
			// 9: select list, "o<TAB>id<TAB>flags<TAB>label" or
			// "g<TAB><TAB><TAB>label".
			BPopUpMenu* menu = new BPopUpMenu("page", false, false);
			int32 start = 0;
			while (start <= all.Length()) {
				int32 end = all.FindFirst('\n', start);
				if (end < 0)
					end = all.Length();
				BString line;
				all.CopyInto(line, start, end - start);
				start = end + 1;
				if (line == "-") {
					menu->AddSeparatorItem();
					continue;
				}
				int32 t1 = line.FindFirst('\t');
				int32 t2 = t1 >= 0 ? line.FindFirst('\t', t1 + 1) : -1;
				int32 t3 = t2 >= 0 ? line.FindFirst('\t', t2 + 1) : -1;
				if (t3 < 0)
					continue;
				BString label;
				line.CopyInto(label, t3 + 1, line.Length() - t3 - 1);
				if (line[0] == 'g') {
					BMenuItem* heading = new BMenuItem(label.String(), NULL);
					heading->SetEnabled(false);
					menu->AddItem(heading);
					continue;
				}
				BString id;
				BString flags;
				line.CopyInto(id, t1 + 1, t2 - t1 - 1);
				line.CopyInto(flags, t2 + 1, t3 - t2 - 1);
				BMessage* choice = new BMessage('svSl');
				choice->AddInt32("id", atoi(id.String()));
				BMenuItem* item = new BMenuItem(label.String(), choice);
				if (line[0] == 'i')
					item->SetEnabled(flags == "1");
				else {
					item->SetEnabled(flags.FindFirst('d') < 0);
					item->SetMarked(flags.FindFirst('s') >= 0);
				}
				menu->AddItem(item);
			}
			BMenuItem* chosen = run_menu(view, menu, x, y);
			int32 id = -1;
			if (chosen != NULL && chosen->Message() != NULL)
				chosen->Message()->FindInt32("id", &id);
			delete menu;
			*accept = id >= 0 ? 1 : 0;
			*value = id;
			break;
		}
		default:
			return B_NOT_SUPPORTED;
	}
	if (reply.Length() > 0) {
		*answer = strdup(reply.String());
		// The reply may hold a password: clear the copy here.
		wipe_string(reply);
	}
	return B_OK;
}


extern "C" void
r5_servo_free_answer(char* answer)
{
	if (answer == NULL)
		return;
	memset(answer, 0, strlen(answer));
	free(answer);
}


extern "C" status_t
r5_servo_show_control(r5_servo* engine, int64 id, BView* view, int32 kind,
	const char* text, float x, float y, const char* okLabel,
	const char* cancelLabel)
{
	if (engine == NULL)
		return B_BAD_VALUE;
	r5_servo_control_labels labels = { okLabel != NULL ? okLabel : "OK",
		cancelLabel != NULL ? cancelLabel : "Cancel", "User name:",
		"Password:", "Sign in", "Sign in to %s" };
	int32 accept = 0;
	int32 value = -1;
	char* answer = NULL;
	status_t status = r5_servo_ask_control(view, kind, text, x, y, &labels,
		&accept, &value, &answer);
	if (status != B_OK)
		return status;
	status = r5_servo_control_answer(engine, id, accept, value, answer);
	r5_servo_free_answer(answer);
	return status;
}


extern "C" status_t
r5_servo_input_message(r5_servo* engine, const BMessage* message, float x,
	float y)
{
	if (engine == NULL || message == NULL)
		return B_BAD_VALUE;
	// GetPoint(): FindPoint() would set (0, 0) when the field is missing (a
	// wheel message has no be:view_where).
	BPoint where = message->GetPoint("be:view_where", BPoint(x, y));
	switch (message->what) {
		case B_MOUSE_DOWN:
		case B_MOUSE_UP:
		{
			// "buttons" holds the buttons still down after the change.
			int32 buttons = 0;
			message->FindInt32("buttons", &buttons);
			return r5_servo_mouse_buttons(engine, where.x, where.y,
				(uint32)buttons);
		}
		case B_MOUSE_MOVED:
			return r5_servo_mouse_move(engine, where.x, where.y);
		case B_MOUSE_WHEEL_CHANGED:
		{
			float dx = 0;
			float dy = 0;
			message->FindFloat("be:wheel_delta_x", &dx);
			message->FindFloat("be:wheel_delta_y", &dy);
			// Three lines of the plain font per notch.
			float notch = ceilf(be_plain_font->Size() * 4.0f);
			return r5_servo_wheel(engine, where.x, where.y, dx, dy, notch);
		}
		case B_KEY_DOWN:
		case B_UNMAPPED_KEY_DOWN:
		case B_KEY_UP:
		case B_UNMAPPED_KEY_UP:
		{
			const char* bytes = NULL;
			if (message->FindString("bytes", &bytes) != B_OK || bytes == NULL
				|| bytes[0] == '\0')
				return B_BAD_VALUE;
			int32 modifiers = 0;
			message->FindInt32("modifiers", &modifiers);
			const char* keyText = bytes;
			char rawText[2] = { 0, 0 };
			int32 named = R5_SERVO_KEY_NONE;
			if (bytes[1] == '\0') {
				switch (bytes[0]) {
					case B_ENTER: named = R5_SERVO_KEY_ENTER; break;
					case B_BACKSPACE: named = R5_SERVO_KEY_BACKSPACE; break;
					case B_TAB: named = R5_SERVO_KEY_TAB; break;
					case B_ESCAPE: named = R5_SERVO_KEY_ESCAPE; break;
					case B_DELETE: named = R5_SERVO_KEY_DELETE; break;
					case B_LEFT_ARROW: named = R5_SERVO_KEY_LEFT; break;
					case B_RIGHT_ARROW: named = R5_SERVO_KEY_RIGHT; break;
					case B_UP_ARROW: named = R5_SERVO_KEY_UP; break;
					case B_DOWN_ARROW: named = R5_SERVO_KEY_DOWN; break;
					case B_HOME: named = R5_SERVO_KEY_HOME; break;
					case B_END: named = R5_SERVO_KEY_END; break;
					case B_PAGE_UP: named = R5_SERVO_KEY_PAGE_UP; break;
					case B_PAGE_DOWN: named = R5_SERVO_KEY_PAGE_DOWN; break;
					case B_INSERT: named = R5_SERVO_KEY_INSERT; break;
					case B_FUNCTION_KEY:
					{
						int32 key = 0;
						message->FindInt32("key", &key);
						if (key < B_F1_KEY || key > B_F12_KEY)
							return B_NOT_SUPPORTED;
						named = R5_SERVO_KEY_F1 + (key - B_F1_KEY);
						break;
					}
					default:
						if ((uint8)bytes[0] < 0x20) {
							// With Control held Haiku gives a control
							// character; the page needs the letter
							// ("raw_char", as BWindow reads it).
							int32 raw = 0;
							message->FindInt32("raw_char", &raw);
							if (raw < 0x20 || raw >= 0x7f
								|| (modifiers & (B_CONTROL_KEY
									| B_COMMAND_KEY)) == 0)
								return B_NOT_SUPPORTED;
							rawText[0] = (char)raw;
							keyText = rawText;
						}
						break;
				}
			}
			bool down = message->what == B_KEY_DOWN
				|| message->what == B_UNMAPPED_KEY_DOWN;
			return r5_servo_key(engine, down ? 1 : 0, named, keyText,
				servo_modifiers((uint32)modifiers),
				message->HasInt32("be:key_repeat") ? 1 : 0);
		}
		default:
			return B_BAD_VALUE;
	}
}


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
	sPendingStarts.erase(this);
	// Frees the engine; its GL context is kept for reuse, because destroying
	// one aborts in Haiku Mesa's eglDestroyImageKHR.
	// Servo is not shut down here: a program may open a view again later,
	// and Servo cannot be built twice in a process. A program that wants
	// its cookies and logins written calls r5_servo_shutdown() on the
	// view's thread when it is done with every view (R5Book servo-view).
	if (fEngine != NULL)
		r5_servo_destroy(fEngine);
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
	if (sPendingStarts.find(this) != sPendingStarts.end())
		BMessenger(this).SendMessage(kStartMessage);
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
	// Servo and every call into it live on the thread that builds it, and
	// the view is drawn and pumped on its window's thread: start there. A
	// load from another thread (or before the window runs) is queued.
	BWindow* window = Window();
	if (window == NULL || window->Thread() != find_thread(NULL)) {
		PendingStart& pending = sPendingStarts[this];
		pending.document = document != NULL ? document : "";
		pending.isHTML = documentIsHTML;
		if (window != NULL)
			BMessenger(this).SendMessage(kStartMessage);
		return B_OK;
	}
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
	{
		// The GL storage is made once this large; the view never grows past it.
		BScreen screen(window);
		BRect frame = screen.Frame();
		r5_servo_set_max_size((int32)frame.Width() + 1,
			(int32)frame.Height() + 1);
	}
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
	if (fBitmap == NULL)
		return;
	// Never stretched: the frame is drawn at its own size from the top left.
	BRect frame = fBitmap->Bounds();
	DrawBitmap(fBitmap, frame, frame);
	BRect b = Bounds();
	SetHighColor(255, 255, 255);
	if (b.right > frame.right)
		FillRect(BRect(frame.right + 1, b.top, b.right, b.bottom));
	if (b.bottom > frame.bottom)
		FillRect(BRect(b.left, frame.bottom + 1, frame.right, b.bottom));
}


void
BServoView::MouseDown(BPoint where)
{
	MakeFocus(true);
	SetMouseEventMask(B_POINTER_EVENTS, B_NO_POINTER_HISTORY);
	if (fEngine != NULL && Window() != NULL)
		r5_servo_input_message(fEngine, Window()->CurrentMessage(), where.x,
			where.y);
}


void
BServoView::MouseUp(BPoint where)
{
	if (fEngine != NULL && Window() != NULL)
		r5_servo_input_message(fEngine, Window()->CurrentMessage(), where.x,
			where.y);
}


void
BServoView::MouseMoved(BPoint where, uint32 transit, const BMessage* dragMessage)
{
	BView::MouseMoved(where, transit, dragMessage);
	if (fEngine != NULL)
		r5_servo_mouse_move(fEngine, where.x, where.y);
}


void
BServoView::KeyDown(const char* bytes, int32 numBytes)
{
	if (fEngine == NULL || Window() == NULL) {
		BView::KeyDown(bytes, numBytes);
		return;
	}
	r5_servo_input_message(fEngine, Window()->CurrentMessage(), 0, 0);
}


void
BServoView::KeyUp(const char* bytes, int32 numBytes)
{
	if (fEngine == NULL || Window() == NULL) {
		BView::KeyUp(bytes, numBytes);
		return;
	}
	r5_servo_input_message(fEngine, Window()->CurrentMessage(), 0, 0);
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
BServoView::Show()
{
	BView::Show();
	// Shown only when no parent is hidden either; a window that is not
	// shown yet does not count (IsHidden(lookingFrom) with the view itself
	// checks only its own show level).
	bool hidden = false;
	for (const BView* v = this; v != NULL; v = v->Parent()) {
		if (v->IsHidden(v))
			hidden = true;
	}
	if (fEngine != NULL && !hidden)
		r5_servo_set_visible(fEngine, 1);

}


void
BServoView::Hide()
{
	BView::Hide();
	if (fEngine != NULL)
		r5_servo_set_visible(fEngine, 0);
}



void
BServoView::MessageReceived(BMessage* message)
{
	if (message->what == kSpinMessage) {
		// Runs Servo's event loop; copies only a new frame.
		if (fEngine != NULL) {
			r5_servo_pump();
			if (r5_servo_paint(fEngine) == 1)
				_CopyFrame();
		}
		return;
	}
	if (message->what == kStartMessage) {
		std::map<const BServoView*, PendingStart>::iterator found
			= sPendingStarts.find(this);
		if (found != sPendingStarts.end()) {
			PendingStart pending = found->second;
			sPendingStarts.erase(found);
			_Start(pending.document.String(), pending.isHTML);
		}
		return;
	}
	if (message->what == kControlMessage && fEngine != NULL) {
		int32 kind = 0;
		const char* text = "";
		BPoint where(0, 0);
		message->FindInt32("kind", &kind);
		message->FindString("text", &text);
		message->FindPoint("where", &where);
		r5_servo_show_control(fEngine, message->GetInt64("control", 0), this,
			kind, text, where.x, where.y, NULL, NULL);
		return;
	}
	if (fEngine != NULL) {
		// The Edit shortcuts (Alt+C, X, V, A) reach the focus view as these
		// messages (BWindow adds them as shortcuts).
		switch (message->what) {
			case B_COPY:
				r5_servo_editing_action(fEngine, 0);
				return;
			case B_CUT:
				r5_servo_editing_action(fEngine, 1);
				return;
			case B_PASTE:
				r5_servo_editing_action(fEngine, 2);
				return;
			case B_SELECT_ALL:
				r5_servo_key(fEngine, 1, R5_SERVO_KEY_NONE, "a",
					R5_SERVO_MOD_CONTROL, 0);
				r5_servo_key(fEngine, 0, R5_SERVO_KEY_NONE, "a",
					R5_SERVO_MOD_CONTROL, 0);
				return;
		}
	}
	if (message->what == B_MOUSE_WHEEL_CHANGED && fEngine != NULL) {

		BPoint where;
		uint32 buttons;
		GetMouse(&where, &buttons, false);
		r5_servo_input_message(fEngine, message, where.x, where.y);
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
	// Servo gives RGBA bytes; B_RGBA32 is B, G, R, A in memory.
	uint8* dest = (uint8*)fBitmap->Bits();
	int32 stride = fBitmap->BytesPerRow();
	for (int32 y = 0; y < height; y++) {
		const uint32* src = (const uint32*)(rgba + (size_t)y * width * 4);
		uint32* row = (uint32*)(dest + (size_t)y * stride);
		for (int32 x = 0; x < width; x++) {
			uint32 p = B_LENDIAN_TO_HOST_INT32(src[x]);
			row[x] = B_HOST_TO_LENDIAN_INT32((p & 0xff00ff00)
				| ((p & 0x000000ff) << 16) | ((p >> 16) & 0x000000ff));
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
			// Servo already made an engine for the page's new window. This
			// view has no place for it: refuse by destroying it (the page's
			// window.open() then gets no window). Aomi adopts it instead.
			if (event->current != 0)
				r5_servo_destroy((r5_servo*)(addr_t)event->current);
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
		{
			_Report(AOMI_DIALOG, NULL, event->text, 0, event->a, 0, 0);
			// The page waits for an alert, confirm, prompt or select list.
			// Answer it from the message loop, not from inside Servo's pump.
			BMessage control(kControlMessage);
			control.AddInt32("kind", event->a);
			control.AddString("text", event->text != NULL ? event->text : "");
			control.AddPoint("where", BPoint(event->b, event->c));
			control.AddInt64("control", event->current);

			BMessenger(this).SendMessage(&control);
			break;
		}
		default:
			break;
	}
}
