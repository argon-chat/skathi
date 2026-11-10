//! The application's icon in the menu bar: an `NSStatusItem`.
//!
//! AppKit belongs to the main thread, which on macOS is also gpui's and JSC's.
//! Windows gives the tray a thread and a message window of its own; here the
//! main queue is that thread. `set` and `stop` may come from any thread: they
//! admit the change exactly as windows.rs does (the same revision and the same
//! coalescing slot) and wake the main queue, never waiting for it, so a caller
//! on the main thread cannot deadlock and nothing appears before AppKit runs.
//!
//! - **The item** is created by the first model and removed by `set(None)` or
//!   `stop` (`removeStatusItem:`). Its button shows the icon (mac_plan.rs: an
//!   18-point square at 1× and 2×, a template when asked or one grey tone;
//!   without an icon, the application's), the tooltip, and the same text as
//!   its VoiceOver label.
//! - **Clicks** reach one target object per controller, on mouse up of either
//!   button. Without `on_click` the item owns its menu and AppKit opens it at
//!   any click. With one, a primary click is reported, and a secondary click
//!   (or Control-click) opens the menu for the duration of that click
//!   (`performClick:` with the menu attached, then detached again).
//! - **Menu items** are built from the model when it is admitted, with the
//!   revision and their action slot in the `tag`, so the current snapshot
//!   fences a selection from a replaced menu as windows.rs does. A new model
//!   cancels an open menu (windows.rs posts `WM_CANCELMODE`).
//! - **Callbacks** run on the main thread. A Rust callback must not block it;
//!   JS callbacks are posted to the VM by vn-app as on Windows.

use super::{
    mac_plan::{self, Node, Press},
    Callback, Error, Icon, Options,
};
use objc2::{
    define_class, msg_send,
    rc::{autoreleasepool, Retained},
    runtime::{AnyObject, NSObject},
    sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSBitmapImageRep, NSControlStateValueOff, NSControlStateValueOn, NSEventMask,
    NSEventModifierFlags, NSEventType, NSImage, NSMenu, NSMenuItem, NSRunningApplication,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSCopying, NSData, NSSize, NSString};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    panic::AssertUnwindSafe,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct Controller {
    /// This controller's item in [`ITEMS`].
    id: u64,
    me: Weak<Controller>,
    stopping: AtomicBool,
    revision: AtomicU64,
    pending: Mutex<Option<(u64, Option<Options>)>>,
    /// A drain is queued on the main queue; later updates only replace
    /// `pending` (they coalesce, as windows.rs's do).
    scheduled: AtomicBool,
}
impl Controller {
    pub(super) fn start() -> Result<Arc<Self>, Error> {
        Ok(Arc::new_cyclic(|me| Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            me: me.clone(),
            stopping: AtomicBool::new(false),
            revision: AtomicU64::new(0),
            pending: Mutex::new(None),
            scheduled: AtomicBool::new(false),
        }))
    }
    pub(super) fn set(&self, options: Option<Options>) -> Result<u64, Error> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let mut pending = self.pending.lock().map_err(|_| Error::Closed)?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let revision = self.revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        // Never run a user capture's destructor while holding the queue mutex.
        let discarded = pending.replace((revision, options));
        drop(pending);
        drop(discarded);
        if !self.scheduled.swap(true, Ordering::AcqRel) {
            if let Some(shared) = self.me.upgrade() {
                on_main(move |mtm| {
                    shared.scheduled.store(false, Ordering::Release);
                    apply(mtm, &shared);
                });
            }
        }
        Ok(revision)
    }
    pub(super) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    fn current(&self, revision: u64) -> bool {
        !self.stopping.load(Ordering::Acquire) && revision == self.revision()
    }
    pub(super) fn stop(&self) {
        // Serialize the tombstone with set(), as windows.rs does.
        let discarded = match self.pending.lock() {
            Ok(mut pending) => {
                if !self.stopping.swap(true, Ordering::AcqRel) {
                    self.revision.fetch_add(1, Ordering::AcqRel);
                }
                pending.take()
            }
            Err(_) => {
                self.stopping.store(true, Ordering::Release);
                None
            }
        };
        drop(discarded);
        let id = self.id;
        // At once on the main thread, unless a click of this item is still on
        // the stack (its menu may be tracking): AppKit is left to unwind first.
        match MainThreadMarker::new() {
            Some(mtm) if ACTIVE.with(Cell::get) == 0 => remove(mtm, id),
            _ => on_main(move |mtm| remove(mtm, id)),
        }
    }
}

/// Runs `work` on the main queue, inside an autorelease pool; a panic is
/// logged instead of unwinding into libdispatch.
fn on_main(work: impl FnOnce(MainThreadMarker) + Send + 'static) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        let ran = std::panic::catch_unwind(AssertUnwindSafe(|| {
            autoreleasepool(|_| match MainThreadMarker::new() {
                Some(mtm) => work(mtm),
                None => crate::log_later(
                    "error",
                    "native tray: the main queue ran off the main thread".into(),
                ),
            })
        }));
        if ran.is_err() {
            crate::log_later("error", "native tray update panicked".into());
        }
    });
}

/// An AppKit callback's body; a panic is logged instead of unwinding into
/// AppKit, and `stop` knows not to remove the item under it.
fn guarded(body: impl FnOnce()) {
    struct Active;
    impl Drop for Active {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(active.get().saturating_sub(1)));
        }
    }
    ACTIVE.with(|active| active.set(active.get() + 1));
    let _active = Active;
    if std::panic::catch_unwind(AssertUnwindSafe(body)).is_err() {
        crate::log_later("error", "native tray event panicked".into());
    }
}

/// The model a status item shows, and the action table its menu was built
/// with (mac_plan.rs). Immutable: a menu that is open keeps its own.
struct Snapshot {
    revision: u64,
    options: Options,
    actions: Vec<Option<Callback>>,
}

/// A controller's status item. Main thread only.
struct Native {
    mtm: MainThreadMarker,
    shared: Weak<Controller>,
    status: Retained<NSStatusItem>,
    /// The button's and every menu item's target. Neither retains it.
    target: Retained<Target>,
    snapshot: Rc<Snapshot>,
    menu: Option<Retained<NSMenu>>,
    /// The icon model the button's image was made from.
    icon: Option<Option<Icon>>,
}
impl Native {
    fn new(mtm: MainThreadMarker, shared: &Arc<Controller>, snapshot: Rc<Snapshot>) -> Self {
        let status =
            NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let target = Target::new(mtm, shared.id);
        if let Some(button) = status.button(mtm) {
            // SAFETY: the target answers `press:` and outlives the button's use
            // of it (Native clears it before releasing either).
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(press:)));
            }
            button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
        }
        Self {
            mtm,
            shared: Arc::downgrade(shared),
            status,
            target,
            snapshot,
            menu: None,
            icon: None,
        }
    }
    fn owns_menu(&self) -> bool {
        mac_plan::owns_menu(
            self.snapshot.options.on_click.is_some(),
            self.menu.is_some(),
        )
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        if let Some(menu) = self.menu.take() {
            menu.cancelTracking();
            detach(&menu);
        }
        self.status.setMenu(None);
        if let Some(button) = self.status.button(self.mtm) {
            // SAFETY: clearing the target and action.
            unsafe {
                button.setTarget(None);
                button.setAction(None);
            }
        }
        NSStatusBar::systemStatusBar().removeStatusItem(&self.status);
    }
}

thread_local! {
    /// Every controller's item, by controller id. Main thread only. Taken out
    /// while AppKit is called, so no borrow is held across a nested run loop.
    static ITEMS: RefCell<HashMap<u64, Native>> = RefCell::new(HashMap::new());
    /// Status-item callbacks on the stack.
    static ACTIVE: Cell<usize> = const { Cell::new(0) };
    /// The last failure reported per controller, so a repeated one logs once.
    static REPORTED: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
}

fn report(id: u64, message: String) {
    let repeated = REPORTED.with(|reported| reported.borrow().get(&id) == Some(&message));
    if !repeated {
        crate::log_later("warn", message.clone());
        REPORTED.with(|reported| reported.borrow_mut().insert(id, message));
    }
}

/// Main thread: shows the controller's pending model, or removes its item.
fn apply(mtm: MainThreadMarker, shared: &Arc<Controller>) {
    if shared.stopping.load(Ordering::Acquire) {
        return;
    }
    let command = shared
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.take());
    let Some((revision, options)) = command else {
        return;
    };
    let Some(options) = options else {
        remove(mtm, shared.id);
        return;
    };
    let (nodes, actions) = mac_plan::menu(&options.menu, options.on_quit.as_ref());
    let snapshot = Rc::new(Snapshot {
        revision,
        options,
        actions,
    });
    let existing = ITEMS.with(|items| items.borrow_mut().remove(&shared.id));
    let mut native = match existing {
        Some(mut native) => {
            native.snapshot = snapshot.clone();
            native
        }
        None => Native::new(mtm, shared, snapshot.clone()),
    };
    // Old model's menu first: an open one is closed, as on Windows.
    let old_menu = native.menu.take();
    if let Some(menu) = &old_menu {
        menu.cancelTracking();
        detach(menu);
    }
    native.menu = (!nodes.is_empty()).then(|| build(mtm, "", &nodes, &native.target, revision));
    native.status.setMenu(if native.owns_menu() {
        native.menu.as_deref()
    } else {
        None
    });
    if let Some(button) = native.status.button(mtm) {
        let tooltip = NSString::from_str(&mac_plan::tooltip(&snapshot.options.tooltip, app_name));
        button.setToolTip(Some(&tooltip));
        // An image alone says nothing to VoiceOver.
        // SAFETY: `setAccessibilityLabel:` takes an optional NSString.
        let _: () = unsafe { msg_send![&*button, setAccessibilityLabel: &*tooltip] };
        let icon = snapshot.options.icon.as_ref();
        if native.icon.as_ref().map(Option::as_ref) != Some(icon) {
            match image(mtm, icon) {
                Ok(image) => {
                    button.setImage(Some(&image));
                    native.icon = Some(icon.cloned());
                }
                Err(error) => {
                    report(shared.id, format!("native tray icon: {error}"));
                    // An item without an image cannot be seen or clicked.
                    if button.image().is_none() {
                        button.setImage(app_icon(mtm).as_deref());
                    }
                }
            }
        }
    }
    ITEMS.with(|items| items.borrow_mut().insert(shared.id, native));
    // The replaced menu goes once the new one is attached.
    drop(old_menu);
}

/// Main thread: removes the controller's item, if it has one.
fn remove(_: MainThreadMarker, id: u64) {
    let native = ITEMS.with(|items| items.borrow_mut().remove(&id));
    REPORTED.with(|reported| reported.borrow_mut().remove(&id));
    drop(native);
}

/// The item's button was clicked (on mouse up).
fn pressed(mtm: MainThreadMarker, id: u64) {
    let Some((shared, status, snapshot, has_menu)) = ITEMS.with(|items| {
        let items = items.borrow();
        let native = items.get(&id)?;
        Some((
            native.shared.clone(),
            native.status.clone(),
            native.snapshot.clone(),
            native.menu.is_some(),
        ))
    }) else {
        return;
    };
    let secondary = NSApplication::sharedApplication(mtm)
        .currentEvent()
        .is_some_and(|event| {
            let kind = event.r#type();
            mac_plan::secondary(
                kind == NSEventType::RightMouseUp || kind == NSEventType::RightMouseDown,
                event
                    .modifierFlags()
                    .contains(NSEventModifierFlags::Control),
            )
        });
    match mac_plan::press(secondary, snapshot.options.on_click.is_some(), has_menu) {
        Press::Click => {
            if let Some(callback) = &snapshot.options.on_click {
                fire(&shared, &snapshot, callback);
            }
        }
        Press::Menu => open_menu(mtm, id, &status),
        Press::Nothing => {}
    }
}

/// A secondary click with `on_click`: the item shows its menu for as long as
/// the click's tracking lasts, since AppKit highlights the item and places
/// the menu only for a menu of its own.
fn open_menu(mtm: MainThreadMarker, id: u64, status: &NSStatusItem) {
    let Some(menu) = ITEMS.with(|items| {
        items
            .borrow()
            .get(&id)
            .and_then(|native| native.menu.clone())
    }) else {
        return;
    };
    status.setMenu(Some(&menu));
    if let Some(button) = status.button(mtm) {
        // SAFETY: a click without a sender. Returns once the menu has closed;
        // the registry is not borrowed meanwhile, so a model can arrive.
        unsafe { button.performClick(None) };
    }
    // The item as its model is now (removed, replaced, or the same).
    let owned = ITEMS.with(|items| {
        let items = items.borrow();
        let native = items.get(&id)?;
        native.owns_menu().then(|| native.menu.clone()).flatten()
    });
    status.setMenu(owned.as_deref());
}

/// A menu item was chosen.
fn selected(id: u64, tag: isize) {
    let Some((shared, snapshot)) = ITEMS.with(|items| {
        let items = items.borrow();
        let native = items.get(&id)?;
        Some((native.shared.clone(), native.snapshot.clone()))
    }) else {
        return;
    };
    let callback = mac_plan::action_of(tag, snapshot.revision)
        .and_then(|action| snapshot.actions.get(action))
        .and_then(Option::as_ref);
    if let Some(callback) = callback {
        fire(&shared, &snapshot, callback);
    }
}

/// Runs a callback of the current model only (windows.rs's `selected`).
fn fire(shared: &Weak<Controller>, snapshot: &Snapshot, callback: &Callback) {
    if shared
        .upgrade()
        .is_some_and(|shared| shared.current(snapshot.revision))
    {
        super::invoke(callback);
    }
}

struct TargetIvars {
    id: u64,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Target does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VueNativeTrayTarget"]
    #[ivars = TargetIvars]
    struct Target;

    impl Target {
        /// The status item's button, on mouse up of either button.
        #[unsafe(method(press:))]
        fn press(&self, _sender: Option<&AnyObject>) {
            let (mtm, id) = (self.mtm(), self.ivars().id);
            guarded(|| pressed(mtm, id));
        }

        /// A menu item; its tag is mac_plan.rs's.
        #[unsafe(method(select:))]
        fn select(&self, sender: Option<&NSMenuItem>) {
            let id = self.ivars().id;
            if let Some(tag) = sender.map(NSMenuItem::tag) {
                guarded(|| selected(id, tag));
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, id: u64) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { id });
        // SAFETY: NSObject's initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// The menu for `nodes`. Items are enabled by the model, not by AppKit's
/// responder-chain validation.
fn build(
    mtm: MainThreadMarker,
    title: &str,
    nodes: &[Node],
    target: &Target,
    revision: u64,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    menu.setAutoenablesItems(false);
    let none = NSString::from_str("");
    for node in nodes {
        let item = match node {
            Node::Separator => NSMenuItem::separatorItem(mtm),
            Node::Item {
                label,
                enabled,
                checked,
                action,
            } => {
                // SAFETY: `select:` is the target's action; the target
                // outlives the menu (Native detaches it first).
                let item = unsafe {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(label),
                        Some(sel!(select:)),
                        &none,
                    );
                    item.setTarget(Some(target));
                    item
                };
                item.setTag(mac_plan::tag(revision, *action));
                item.setEnabled(*enabled);
                item.setState(if *checked {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                item
            }
            Node::Submenu {
                label,
                enabled,
                items,
            } => {
                // SAFETY: no action; the submenu opens it.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(label),
                        None,
                        &none,
                    )
                };
                item.setSubmenu(Some(&build(mtm, label, items, target, revision)));
                item.setEnabled(*enabled);
                item
            }
        };
        menu.addItem(&item);
    }
    menu
}

/// Clears every item's target, so a menu AppKit still holds cannot reach a
/// released target.
fn detach(menu: &NSMenu) {
    for item in menu.itemArray().iter() {
        // SAFETY: clearing the target.
        unsafe { item.setTarget(None) };
        if let Some(submenu) = item.submenu() {
            detach(&submenu);
        }
    }
}

/// The button's image: the model's icon at 1× and 2×, or the application's.
fn image(mtm: MainThreadMarker, icon: Option<&Icon>) -> Result<Retained<NSImage>, String> {
    let Some(icon) = icon else {
        return app_icon(mtm).ok_or_else(|| "the application has no icon".into());
    };
    let side = f64::from(mac_plan::SIDE_PT);
    let size = NSSize::new(side, side);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    for scale in mac_plan::SCALES {
        let data = NSData::with_bytes(&mac_plan::png(icon, scale)?);
        let rep = NSBitmapImageRep::imageRepWithData(&data)
            .ok_or("AppKit could not read the tray icon")?;
        // Points, not pixels: the 2× representation is the same size.
        rep.setSize(size);
        image.addRepresentation(&rep);
    }
    image.setTemplate(mac_plan::template(icon));
    Ok(image)
}

/// The application's icon, at the size the item shows it.
fn app_icon(mtm: MainThreadMarker) -> Option<Retained<NSImage>> {
    let icon = NSApplication::sharedApplication(mtm).applicationIconImage()?;
    // A copy: the application's image is shared, and its size is the image's.
    let icon = icon.copy();
    let side = f64::from(mac_plan::SIDE_PT);
    icon.setSize(NSSize::new(side, side));
    Some(icon)
}

/// The bundle's display name (or the process name), for an empty tooltip.
fn app_name() -> Option<String> {
    NSRunningApplication::currentApplication()
        .localizedName()
        .map(|name| name.to_string())
}
