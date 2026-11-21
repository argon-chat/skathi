//! The menu bar a macOS app is expected to have. AppKit performs every item on the key window or the
//! application, so Close goes through the window's close request (`close-requested`, `beforeunload`)
//! and Quit through the terminate request the Dock's Quit sends. A key equivalent reaches the menu
//! only when the page did not `preventDefault()` the keydown.

use gpui::{App, Menu, MenuItem, NoAction, OsAction, SystemMenuType};

pub(crate) fn install(cx: &mut App, name: &str) {
    if !crate::embed::app_menu() {
        return;
    }
    let name = if name.trim().is_empty() {
        "Skathi"
    } else {
        name
    };
    let item = |title: String, action: OsAction| MenuItem::os_action(title, NoAction, action);
    cx.set_menus([
        Menu::new(name.to_string()).items([
            item(format!("About {name}"), OsAction::About),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            item(format!("Hide {name}"), OsAction::Hide),
            item("Hide Others".into(), OsAction::HideOthers),
            item("Show All".into(), OsAction::ShowAll),
            MenuItem::separator(),
            item(format!("Quit {name}"), OsAction::Quit),
        ]),
        Menu::new("File").items([item("Close Window".into(), OsAction::Close)]),
        // AppKit retitles it Exit Full Screen while the key window is in full screen.
        Menu::new("View").items([item("Enter Full Screen".into(), OsAction::ToggleFullScreen)]),
        Menu::new("Window").items([
            item("Minimize".into(), OsAction::Minimize),
            item("Zoom".into(), OsAction::Zoom),
            MenuItem::separator(),
            item("Bring All to Front".into(), OsAction::BringAllToFront),
        ]),
    ]);
}
