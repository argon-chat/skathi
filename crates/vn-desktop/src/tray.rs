//! Application-owned notification icons. OS callbacks never run inside JS: on
//! Windows they run on the tray's own thread; on macOS on the main thread
//! (which is also the VM's), from AppKit's event dispatch.

use std::sync::{Arc, Mutex, OnceLock, Weak};

pub type Callback = Arc<dyn Fn() + Send + Sync + 'static>;

/// Straight RGBA pixels; the OS owns its own copy after an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// Render the alpha mask in the taskbar's foreground colour (on macOS, a
    /// template image in the menu bar's text colour).
    pub template: bool,
}

impl Icon {
    pub fn rgba(width: u32, height: u32, pixels: impl Into<Arc<[u8]>>) -> Result<Self, Error> {
        let icon = Self {
            width,
            height,
            rgba: pixels.into(),
            template: false,
        };
        icon.validate()?;
        Ok(icon)
    }
    fn validate(&self) -> Result<(), Error> {
        if self.width == 0
            || self.height == 0
            || self.width > 512
            || self.height > 512
            || self
                .width
                .checked_mul(self.height)
                .and_then(|n| n.checked_mul(4))
                .map(|n| n as usize)
                != Some(self.rgba.len())
        {
            return Err(Error::Invalid(
                "tray icon must contain exact RGBA pixels, at most 512 × 512".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub enum MenuItem {
    Action {
        label: String,
        disabled: bool,
        checked: Option<bool>,
        default: bool,
        on_select: Option<Callback>,
    },
    Separator,
    Submenu {
        label: String,
        disabled: bool,
        items: Vec<MenuItem>,
    },
    Quit {
        label: Option<String>,
        disabled: bool,
    },
}
impl MenuItem {
    pub fn action(label: impl Into<String>, on_select: impl Fn() + Send + Sync + 'static) -> Self {
        Self::Action {
            label: label.into(),
            disabled: false,
            checked: None,
            default: false,
            on_select: Some(Arc::new(on_select)),
        }
    }
    pub fn submenu(label: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            disabled: false,
            items,
        }
    }
    pub fn quit() -> Self {
        Self::Quit {
            label: None,
            disabled: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct Options {
    /// None uses the executable's icon, with the system icon as a fallback.
    pub icon: Option<Icon>,
    /// At most 127 UTF-16 units; empty uses the executable's name.
    pub tooltip: String,
    pub menu: Vec<MenuItem>,
    /// A primary click opens the menu when this is absent.
    pub on_click: Option<Callback>,
    /// The application decides how to quit. The backend never exits a process.
    pub on_quit: Option<Callback>,
}
impl Options {
    pub fn validate(&self) -> Result<(), Error> {
        if self.tooltip.contains('\0') || self.tooltip.encode_utf16().count() > 127 {
            return Err(Error::Invalid(
                "tray tooltip must be NUL-free and at most 127 UTF-16 units".into(),
            ));
        }
        if let Some(icon) = &self.icon {
            icon.validate()?;
        }
        let mut count = 0;
        validate_menu(&self.menu, 0, &mut count, self.on_quit.is_some())
    }
}
fn validate_menu(
    items: &[MenuItem],
    depth: usize,
    count: &mut usize,
    can_quit: bool,
) -> Result<(), Error> {
    if depth > 8 {
        return Err(Error::Invalid("tray menu nesting exceeds 8".into()));
    }
    for item in items {
        *count += 1;
        if *count > 512 {
            return Err(Error::Invalid("tray menu exceeds 512 items".into()));
        }
        let label = match item {
            MenuItem::Action { label, .. } | MenuItem::Submenu { label, .. } => {
                Some(label.as_str())
            }
            MenuItem::Quit { label, .. } => {
                if !can_quit {
                    return Err(Error::Invalid(
                        "a quit menu item requires Options::on_quit".into(),
                    ));
                }
                label.as_deref()
            }
            MenuItem::Separator => None,
        };
        if label.is_some_and(|label| {
            label.is_empty() || label.contains('\0') || label.encode_utf16().count() > 512
        }) {
            return Err(Error::Invalid(
                "tray menu labels must be nonempty, NUL-free and at most 512 UTF-16 units".into(),
            ));
        }
        if let MenuItem::Submenu { items, .. } = item {
            validate_menu(items, depth + 1, count, can_quit)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    Closed,
    Invalid(String),
    Platform(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("native tray is not implemented on this platform"),
            Self::Closed => f.write_str("native tray has stopped"),
            Self::Invalid(message) | Self::Platform(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Controller;
// The status item's AppKit half; what it is told is decided in mac_plan.rs,
// unit-tested on every OS.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::Controller;
#[cfg(any(target_os = "macos", test))]
mod mac_plan;
#[cfg(not(any(windows, target_os = "macos")))]
struct Controller;
#[cfg(not(any(windows, target_os = "macos")))]
impl Controller {
    fn start() -> Result<Arc<Self>, Error> {
        Err(Error::Unsupported)
    }
    fn set(&self, _: Option<Options>) -> Result<u64, Error> {
        Err(Error::Unsupported)
    }
    fn revision(&self) -> u64 {
        0
    }
    fn stop(&self) {}
}
struct Owner(Arc<Controller>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Cloneable app-owned handle. The final owner removes the icon and stops its pump.
#[derive(Clone)]
pub struct Tray {
    owner: Arc<Owner>,
}
static APP: OnceLock<Mutex<Weak<Owner>>> = OnceLock::new();
impl Tray {
    pub const fn is_available() -> bool {
        cfg!(any(windows, target_os = "macos"))
    }
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            owner: Arc::new(Owner(Controller::start()?)),
        })
    }
    /// The same icon for the application's Rust and JavaScript configuration.
    pub fn app() -> Result<Self, Error> {
        let mut slot = APP
            .get_or_init(|| Mutex::new(Weak::new()))
            .lock()
            .map_err(|_| Error::Closed)?;
        if let Some(owner) = slot.upgrade() {
            return Ok(Self { owner });
        }
        let tray = Self::new()?;
        *slot = Arc::downgrade(&tray.owner);
        Ok(tray)
    }
    pub fn existing_app() -> Option<Self> {
        let owner = APP.get()?.lock().ok()?.upgrade()?;
        Some(Self { owner })
    }
    /// Queues a complete replacement; None removes the icon. Updates coalesce.
    /// The returned revision fences events from a dismissed/replaced menu.
    pub fn set(&self, options: Option<Options>) -> Result<u64, Error> {
        if let Some(options) = &options {
            options.validate()?;
        }
        self.owner.0.set(options)
    }
    pub fn revision(&self) -> u64 {
        self.owner.0.revision()
    }
    pub fn stop(&self) {
        self.owner.0.stop();
    }
    pub fn downgrade(&self) -> WeakTray {
        WeakTray(Arc::downgrade(&self.owner))
    }
}
#[derive(Clone)]
pub struct WeakTray(Weak<Owner>);
impl WeakTray {
    pub fn upgrade(&self) -> Option<Tray> {
        self.0.upgrade().map(|owner| Tray { owner })
    }
}

pub(super) fn invoke(callback: &Callback) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback())).is_err() {
        crate::log_later("error", "native tray callback panicked".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_validation_bounds_state_without_opening_the_desktop() {
        let mut options = Options {
            menu: vec![
                MenuItem::action("Open", || {}),
                MenuItem::Separator,
                MenuItem::submenu(
                    "Status",
                    vec![MenuItem::Action {
                        label: "Muted".into(),
                        disabled: true,
                        checked: Some(true),
                        default: false,
                        on_select: None,
                    }],
                ),
                MenuItem::quit(),
            ],
            on_quit: Some(Arc::new(|| {})),
            ..Default::default()
        };
        assert!(options.validate().is_ok());
        options.on_quit = None;
        assert!(options.validate().is_err());
        options.menu.clear();
        options.tooltip = "😀".repeat(64);
        assert!(options.validate().is_err());
        options.tooltip.clear();
        options.menu = vec![MenuItem::Separator; 513];
        assert!(options.validate().is_err());
        options.menu = vec![MenuItem::action("", || {})];
        assert!(options.validate().is_err());
    }
    #[test]
    fn icon_validation_checks_exact_lengths_before_platform_calls() {
        assert!(Icon::rgba(2, 2, vec![0; 16]).is_ok());
        assert!(Icon::rgba(2, 2, vec![0; 15]).is_err());
        assert!(Icon::rgba(0, 2, vec![]).is_err());
        assert!(Icon::rgba(u32::MAX, u32::MAX, vec![]).is_err());
    }
}
