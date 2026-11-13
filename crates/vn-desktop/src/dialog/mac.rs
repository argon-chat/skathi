//! `NSOpenPanel` / `NSSavePanel`, `NSAlert` and `NSColorPanel`, for a `vn-dialog` thread (mod.rs).
//!
//! AppKit panels belong to the main thread, which is also the frame thread and JSC's. The dialog
//! thread hands the panel to the main queue and waits on a channel; nothing modal runs on the main
//! thread (no `runModal`), so gpui and JS keep running while a panel is open:
//!
//! - with an owner window on screen, the panel is a sheet on it, modal to that window only;
//! - without one (no owner, or it is minimised, hidden or already showing a sheet), a file panel
//!   is modeless. A question needs its window: it is a sheet on the owner even when minimised,
//!   and with no owner at all it is dismissed (ask.rs logs it).
//!
//! The owner (`Owner::hwnd`) is the `NSWindow` resolved from gpui's `NSView`
//! ([`ns_window_of_view`]); being a pointer captured earlier, the main thread uses it only after
//! finding it in `[NSApp windows]`.
//!
//! Cancellation: the dialog thread checks its owner every 100 ms; once it is closing, the main
//! thread ends the panel as its Cancel button would. Every completion handler orders its panel
//! out before answering, so when the thread returns (`is_idle`) no sheet is attached to the owner.
//! After [`GIVE_UP`] the thread stops waiting, so a wedged main thread cannot keep a window open.
//!
//! The colour picker is the shared `NSColorPanel`: not modal, no OK button. The colour it shows
//! when it closes is the answer, an unchanged colour is a cancel, and one is open app-wide.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::{autoreleasepool, Retained, Weak};
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSApplication, NSButton, NSColor, NSColorPanel, NSColorSpace, NSModalResponse,
    NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSPopUpButton, NSRequestUserAttentionType, NSSavePanel,
    NSTextField, NSView, NSWindow, NSWindowWillCloseNotification,
};
use objc2_foundation::{
    NSArray, NSFileManager, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSearchPathDirectory, NSSearchPathDomainMask, NSSize, NSString, NSUserDefaults, NSURL,
};

use super::ask::label_of;
use super::mac_plan::{self, Format};
use super::{Ask, Cancellation, Kind, Known, Native, Start};
use crate::Fail;

// `UTType` is looked up by name; linking makes sure its framework is loaded
// (macOS 11+, below the 13.0 deployment target).
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

const POLL: Duration = Duration::from_millis(100);
/// How long a closing owner waits for AppKit to take the panel away.
const GIVE_UP: Duration = Duration::from_secs(3);

type Answer<T> = Result<Option<T>, Fail>;
type Reply<T> = mpsc::Sender<Answer<T>>;

/// A dialog on screen, as the main thread knows it.
struct Live {
    /// Ends it as its own Cancel would; its completion handler answers.
    close: Rc<dyn Fn()>,
    /// Takes it off screen and answers `None` without waiting for AppKit.
    force: Box<dyn FnOnce()>,
    /// `close` has been tried once already.
    asked: bool,
}

thread_local! {
    /// Main thread only: the dialogs a closing owner may have to end.
    static LIVE: RefCell<HashMap<u64, Live>> = RefCell::new(HashMap::new());
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Registered before the panel begins, so a completion handler that AppKit
/// calls at once (`NSModalResponseAbort`) finds it to forget.
fn register(id: u64, close: Rc<dyn Fn()>, force: Box<dyn FnOnce()>) {
    LIVE.with(|l| l.borrow_mut().insert(id, Live { close, force, asked: false }));
}

/// The dialog has answered: nothing is left to close.
fn forget(id: u64) {
    // Dropped outside the borrow: the closures release AppKit objects.
    let gone = LIVE.with(|l| l.borrow_mut().remove(&id));
    drop(gone);
}

/// Main thread: dialog `id`'s owner is closing. The first request ends the
/// dialog as Cancel would; a later one forces it. AppKit is called outside
/// the borrow, since ending a panel runs its completion handler (`forget`).
fn close(id: u64) {
    enum Step {
        Ask(Rc<dyn Fn()>),
        Force(Box<dyn FnOnce()>),
    }
    let step = LIVE.with(|l| {
        let mut l = l.borrow_mut();
        let live = l.get_mut(&id)?;
        if live.asked {
            l.remove(&id).map(|live| Step::Force(live.force))
        } else {
            live.asked = true;
            Some(Step::Ask(live.close.clone()))
        }
    });
    match step {
        Some(Step::Ask(f)) => f(),
        Some(Step::Force(f)) => f(),
        None => {}
    }
}

/// Runs `show` on the main queue and waits for the dialog's answer, ending
/// the dialog if its owner closes meanwhile (module docs).
fn run<T: Send + 'static>(
    what: &'static str,
    cancel: &Cancellation,
    show: impl FnOnce(MainThreadMarker, u64, Reply<T>) -> Result<(), Fail> + Send + 'static,
) -> Answer<T> {
    if cancel.is_requested() {
        return Ok(None);
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    let gate = cancel.clone();
    dispatch2::DispatchQueue::main().exec_async(move || {
        let failed = tx.clone();
        // A panic must not unwind into libdispatch.
        let shown = std::panic::catch_unwind(AssertUnwindSafe(|| {
            if gate.is_requested() {
                let _ = tx.send(Ok(None));
                return Ok(());
            }
            autoreleasepool(|_| match MainThreadMarker::new() {
                Some(mtm) => show(mtm, id, tx),
                None => Err(Fail::new("InvalidStateError", "the main queue ran off the main thread").code("EIO")),
            })
        }));
        let failure = match shown {
            Ok(Ok(())) => return,
            Ok(Err(f)) => f,
            Err(_) => {
                forget(id);
                Fail::new("InvalidStateError", format!("the {what} panel could not be shown")).code("EIO")
            }
        };
        let _ = failed.send(Err(failure));
    });
    let mut closing: Option<Instant> = None;
    loop {
        match rx.recv_timeout(POLL) {
            Ok(answer) => return answer,
            // Every holder of the reply went away without one.
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !cancel.is_requested() {
            continue;
        }
        let since = *closing.get_or_insert_with(Instant::now);
        if since.elapsed() >= GIVE_UP {
            crate::log_later("dialog", format!("[dialog] the {what} panel did not close with its window; no longer waiting for it"));
            return Ok(None);
        }
        dispatch2::DispatchQueue::main().exec_async(move || close(id));
    }
}

/// Runs a completion handler's body; a panic answers with a failure instead
/// of unwinding into AppKit.
fn guarded<T>(reply: &Reply<T>, body: impl FnOnce() -> Answer<T>) {
    let answer = std::panic::catch_unwind(AssertUnwindSafe(body))
        .unwrap_or_else(|_| Err(Fail::new("InvalidStateError", "the dialog's answer could not be read").code("EIO")));
    let _ = reply.send(answer);
}

/// The `NSWindow` of gpui's `NSView` (its raw window handle): a dialog
/// owner. Main thread only; `None` off it, or for a view in no window.
pub fn ns_window_of_view(ns_view: *mut c_void) -> Option<isize> {
    MainThreadMarker::new()?;
    // SAFETY: gpui's raw handle is its live `NSView`, read on the main thread.
    let view = unsafe { (ns_view as *const NSView).as_ref() }?;
    view.window().map(|w| Retained::as_ptr(&w) as isize)
}

/// The owner's `NSWindow`, if it is still one of this app's windows.
fn window_of(mtm: MainThreadMarker, owner: isize) -> Option<Retained<NSWindow>> {
    if owner == 0 {
        return None;
    }
    NSApplication::sharedApplication(mtm).windows().iter().find(|w| Retained::as_ptr(w) as isize == owner)
}

/// A window a file panel can be a sheet on: on screen, with no sheet already.
fn sheet_parent(mtm: MainThreadMarker, owner: isize) -> Option<Retained<NSWindow>> {
    window_of(mtm, owner).filter(|w| w.isVisible() && !w.isMiniaturized() && w.attachedSheet().is_none())
}

/// A known folder's path, from `NSFileManager` (thread-safe), else under
/// `$HOME`.
pub(super) fn known_folder(k: Known) -> Option<PathBuf> {
    let dir = match k {
        Known::Desktop => NSSearchPathDirectory::DesktopDirectory,
        Known::Documents => NSSearchPathDirectory::DocumentDirectory,
        Known::Downloads => NSSearchPathDirectory::DownloadsDirectory,
        Known::Music => NSSearchPathDirectory::MusicDirectory,
        Known::Pictures => NSSearchPathDirectory::PicturesDirectory,
        Known::Videos => NSSearchPathDirectory::MoviesDirectory,
    };
    let found = autoreleasepool(|_| {
        NSFileManager::defaultManager()
            .URLsForDirectory_inDomains(dir, NSSearchPathDomainMask::UserDomainMask)
            .firstObject()
            .and_then(|url| url.to_file_path())
    });
    found.or_else(|| std::env::var_os("HOME").map(|home| mac_plan::home_folder(Path::new(&home), k)))
}

/// Shows the panel for `n`, a sheet on `owner` when it can be. `Ok(None)` is
/// a dismissal, including the one the owner's closing forces.
pub(super) fn pick(n: &Native, owner: isize, cancel: &Cancellation) -> Answer<Vec<PathBuf>> {
    let n = n.clone();
    run("file", cancel, move |mtm, id, reply| show_panel(mtm, id, &n, owner, reply))
}

/// `UTType`s for `exts`; an empty array (every file) for `None`.
fn content_types(exts: Option<&[String]>) -> Retained<NSArray<AnyObject>> {
    let (Some(exts), Some(cls)) = (exts, AnyClass::get(c"UTType")) else {
        return NSArray::new();
    };
    let types: Vec<Retained<AnyObject>> = exts
        .iter()
        .filter_map(|e| -> Option<Retained<AnyObject>> {
            // An unknown extension still gets a (dynamic) type that matches it.
            unsafe { msg_send![cls, typeWithFilenameExtension: &*NSString::from_str(e)] }
        })
        .collect();
    NSArray::from_retained_slice(&types)
}

/// `allowedContentTypes` (macOS 11+); empty allows every file.
fn set_allowed(panel: &NSSavePanel, types: &NSArray<AnyObject>) {
    unsafe {
        let _: () = msg_send![panel, setAllowedContentTypes: types];
    }
}

struct FormatIvars {
    panel: Weak<NSSavePanel>,
    /// One per pop-up entry, in its order.
    types: Vec<Retained<NSArray<AnyObject>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and FormatTarget
    // does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VueNativeDialogFormatTarget"]
    #[ivars = FormatIvars]
    struct FormatTarget;

    impl FormatTarget {
        /// The "Format:" pop-up's action: the panel takes the entry's types.
        #[unsafe(method(formatChanged:))]
        fn format_changed(&self, sender: &NSPopUpButton) {
            let ivars = self.ivars();
            let types = usize::try_from(sender.indexOfSelectedItem()).ok().and_then(|i| ivars.types.get(i));
            if let (Some(panel), Some(types)) = (ivars.panel.load(), types) {
                set_allowed(&panel, types);
            }
        }
    }
);

impl FormatTarget {
    fn new(mtm: MainThreadMarker, panel: &NSSavePanel, types: Vec<Retained<NSArray<AnyObject>>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FormatIvars { panel: Weak::new(panel), types });
        unsafe { msg_send![super(this), init] }
    }
}

/// "Format: [pop-up]", the accessory view Chromium puts on a Mac panel.
fn format_view(mtm: MainThreadMarker, formats: &[Format], target: &FormatTarget) -> Retained<NSView> {
    let label = NSTextField::labelWithString(&NSString::from_str("Format:"), mtm);
    let popup = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
    for f in formats {
        popup.addItemWithTitle(&NSString::from_str(&f.name));
    }
    popup.selectItemAtIndex(0);
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(sel!(formatChanged:)));
    }
    label.sizeToFit();
    popup.sizeToFit();
    let (l, p) = (label.frame().size, popup.frame().size);
    let (gap, pad) = (8.0, 12.0);
    let popup_width = p.width.max(160.0);
    let height = l.height.max(p.height) + 2.0 * pad;
    let width = pad + l.width + gap + popup_width + pad;
    let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)));
    label.setFrameOrigin(NSPoint::new(pad, (height - l.height) / 2.0));
    popup.setFrame(NSRect::new(NSPoint::new(pad + l.width + gap, (height - p.height) / 2.0), NSSize::new(popup_width, p.height)));
    view.addSubview(&label);
    view.addSubview(&popup);
    view
}

/// The picked paths, from `URLs` (open) or `URL` (save).
fn picked(panel: &NSSavePanel, open: Option<&NSOpenPanel>) -> Result<Vec<PathBuf>, Fail> {
    let urls: Vec<Retained<NSURL>> = match open {
        Some(open) => open.URLs().to_vec(),
        None => panel.URL().into_iter().collect(),
    };
    urls.iter()
        .map(|url| url.to_file_path().ok_or_else(|| Fail::new("InvalidStateError", "a picked item is not a file path").code("EIO")))
        .collect()
}

fn show_panel(mtm: MainThreadMarker, id: u64, n: &Native, owner: isize, reply: Reply<Vec<PathBuf>>) -> Result<(), Fail> {
    let open = (n.kind != Kind::Save).then(|| NSOpenPanel::openPanel(mtm));
    let panel: Retained<NSSavePanel> = match &open {
        Some(open) => Retained::into_super(open.clone()),
        None => NSSavePanel::savePanel(mtm),
    };
    if let Some(open) = &open {
        open.setCanChooseFiles(n.kind == Kind::Open);
        open.setCanChooseDirectories(n.kind == Kind::Folder);
        open.setAllowsMultipleSelection(n.multiple);
        open.setResolvesAliases(true);
        // A bundle is a folder here, as on Windows: never one opaque "file"
        // that the stat after the pick would refuse.
        panel.setTreatsFilePackagesAsDirectories(true);
    }
    panel.setCanCreateDirectories(n.kind != Kind::Open);
    if n.kind == Kind::Save {
        if let Some(name) = &n.suggested_name {
            panel.setNameFieldStringValue(&NSString::from_str(name));
        }
    }

    // The client GUID's remembered folder, kept in the app's defaults.
    let defaults = NSUserDefaults::standardUserDefaults();
    let key = NSString::from_str(&mac_plan::remembered_key(n.client));
    let remembered = defaults.stringForKey(&key).map(|s| PathBuf::from(s.to_string())).filter(|p| p.is_dir());
    let start = match &n.start {
        Some(Start::Known(k)) => known_folder(*k),
        Some(Start::Path(p)) => Some(p.clone()),
        None => None,
    };
    if let Some(url) = mac_plan::start_folder(start, n.force_start, remembered).and_then(NSURL::from_directory_path) {
        panel.setDirectoryURL(Some(&url));
    }

    let formats = mac_plan::formats(n);
    let types: Vec<_> = formats.iter().map(|f| content_types(f.exts.as_deref())).collect();
    if let Some(first) = types.first() {
        set_allowed(&panel, first);
    }
    let target = (formats.len() > 1).then(|| {
        let target = FormatTarget::new(mtm, &panel, types);
        panel.setAccessoryView(Some(&format_view(mtm, &formats, &target)));
        if let Some(open) = &open {
            open.setAccessoryViewDisclosed(true);
        }
        target
    });

    let parent = sheet_parent(mtm, owner);
    let done = {
        let (panel, reply) = (panel.clone(), reply.clone());
        RcBlock::new(move |response: NSModalResponse| {
            // The pop-up's target is weak: the handler keeps it alive.
            let _target = &target;
            guarded(&reply, || {
                // Off screen before the answer: the owner may go right after.
                panel.orderOut(None);
                forget(id);
                if response != NSModalResponseOK {
                    return Ok(None);
                }
                if let Some(dir) = panel.directoryURL().and_then(|u| u.to_file_path()) {
                    let dir = NSString::from_str(&dir.to_string_lossy());
                    unsafe { defaults.setObject_forKey(Some(&dir), &key) };
                }
                picked(&panel, open.as_deref()).map(Some)
            });
        })
    };
    let close: Rc<dyn Fn()> = {
        let panel = panel.clone();
        Rc::new(move || unsafe { panel.cancel(None) })
    };
    let force: Box<dyn FnOnce()> = {
        let (panel, parent) = (panel.clone(), parent.clone());
        Box::new(move || {
            if let Some(parent) = &parent {
                parent.endSheet_returnCode(&panel, NSModalResponseCancel);
            }
            panel.orderOut(None);
            let _ = reply.send(Ok(None));
        })
    };
    register(id, close, force);
    match &parent {
        Some(parent) => panel.beginSheetModalForWindow_completionHandler(parent, &done),
        None => panel.beginWithCompletionHandler(&done),
    }
    Ok(())
}

/// Asks `q` in an `NSAlert` sheet on `owner`: the index of the button
/// pressed, or `Ok(None)` when the owner's closing ends it. An alert has no
/// close box; Esc presses a button titled "Cancel", as AppKit assigns it.
pub(super) fn ask(q: &Ask, owner: isize, cancel: &Cancellation) -> Answer<usize> {
    let q = q.clone();
    run("question", cancel, move |mtm, id, reply| show_alert(mtm, id, &q, owner, reply))
}

fn show_alert(mtm: MainThreadMarker, id: u64, q: &Ask, owner: isize, reply: Reply<usize>) -> Result<(), Fail> {
    let Some(parent) = window_of(mtm, owner).filter(|w| w.isVisible() || w.isMiniaturized()) else {
        return Err(Fail::new("NotSupportedError", "the question has no window on screen to be asked in"));
    };
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str(&q.instruction));
    if !q.content.is_empty() {
        alert.setInformativeText(&NSString::from_str(&q.content));
    }
    let buttons: Vec<Retained<NSButton>> =
        q.buttons.iter().map(|b| alert.addButtonWithTitle(&NSString::from_str(&label_of(b)))).collect();
    let count = buttons.len();
    // AppKit makes the first button the default.
    let default = q.default.min(count.saturating_sub(1));
    if default != 0 {
        buttons[0].setKeyEquivalent(&NSString::from_str(""));
        buttons[default].setKeyEquivalent(&NSString::from_str("\r"));
    }
    let sheet = alert.window();
    sheet.setTitle(&NSString::from_str(&q.title));

    let app = NSApplication::sharedApplication(mtm);
    if parent.isMiniaturized() || !app.isActive() {
        app.requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
    }
    let done = {
        let (alert, reply) = (alert.clone(), reply.clone());
        RcBlock::new(move |response: NSModalResponse| {
            guarded(&reply, || {
                alert.window().orderOut(None);
                forget(id);
                Ok(mac_plan::button_of(response, count))
            });
        })
    };
    let close: Rc<dyn Fn()> = {
        let (parent, sheet) = (parent.clone(), sheet.clone());
        Rc::new(move || parent.endSheet_returnCode(&sheet, NSModalResponseCancel))
    };
    let force: Box<dyn FnOnce()> = Box::new(move || {
        sheet.orderOut(None);
        let _ = reply.send(Ok(None));
    });
    register(id, close, force);
    alert.beginSheetModalForWindow_completionHandler(&parent, Some(&done));
    Ok(())
}

/// The shared panel is the app's, not a window's: one picker at a time.
static COLOR_OPEN: AtomicBool = AtomicBool::new(false);

/// Shows the shared colour panel at `value` (`#rrggbb`) and answers with
/// the colour it shows when the user closes it; `Ok(None)` when unchanged,
/// or when the owner's closing closes it.
pub(super) fn color(value: &str, _owner: isize, cancel: &Cancellation) -> Answer<String> {
    if COLOR_OPEN.swap(true, Ordering::AcqRel) {
        return Err(Fail::new("NotAllowedError", "Color picker already active.").code("EBUSY"));
    }
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            COLOR_OPEN.store(false, Ordering::Release);
        }
    }
    let _open = Open;
    let initial = value.to_string();
    run("colour", cancel, move |mtm, id, reply| show_color(mtm, id, initial, reply))
}

/// `#rrggbb` of what the panel shows; `fallback` for a colour with no sRGB
/// form (a pattern).
fn hex_of_color(color: &NSColor, fallback: &str) -> String {
    match color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) {
        Some(c) => mac_plan::hex_of(c.redComponent(), c.greenComponent(), c.blueComponent()),
        None => fallback.to_string(),
    }
}

type Observer = Rc<RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>>>;

fn stop_observing(observer: &Observer) {
    if let Some(token) = observer.borrow_mut().take() {
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(AsRef::<AnyObject>::as_ref(&*token)) };
    }
}

fn show_color(mtm: MainThreadMarker, id: u64, initial: String, reply: Reply<String>) -> Result<(), Fail> {
    let panel = NSColorPanel::sharedColorPanel(mtm);
    let (r, g, b) = mac_plan::rgb_of(&initial);
    panel.setShowsAlpha(false);
    panel.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0));

    // The observer's token is dropped (and the block with it) on close.
    let observer: Observer = Rc::default();
    let closed = {
        let (panel, observer, reply) = (panel.clone(), observer.clone(), reply.clone());
        RcBlock::new(move |_: NonNull<NSNotification>| {
            guarded(&reply, || {
                stop_observing(&observer);
                forget(id);
                let picked = hex_of_color(&panel.color(), &initial);
                Ok((picked != initial).then_some(picked))
            });
        })
    };
    let token = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSWindowWillCloseNotification),
            Some(&panel),
            None,
            &closed,
        )
    };
    *observer.borrow_mut() = Some(token);

    let close: Rc<dyn Fn()> = {
        let panel = panel.clone();
        Rc::new(move || panel.close())
    };
    let force: Box<dyn FnOnce()> = {
        let panel = panel.clone();
        Box::new(move || {
            stop_observing(&observer);
            panel.orderOut(None);
            let _ = reply.send(Ok(None));
        })
    };
    register(id, close, force);
    panel.makeKeyAndOrderFront(None);
    Ok(())
}
