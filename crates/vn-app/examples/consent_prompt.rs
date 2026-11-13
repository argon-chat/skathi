//! The permission prompt's window on its own, for a look at it
//! (src/consent_window.rs). Windows only.
//!
//! ```text
//! cargo run -p vn-app --example consent_prompt -- [microphone|camera] [ru] [light|dark] [owner]
//!     [--app Name] [--accent 005FB8/60CDFF] [--icon C:\Windows\notepad.exe]
//!     [desktop] [--close-owner-after 2000] [solid]
//! ```

#[cfg(target_os = "windows")]
fn main() {
    use vn_runtime::redjs::webapi::permissions::Capability;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    let capability = if has("camera") { Capability::Camera } else { Capability::Microphone };
    let dark = if has("dark") {
        Some(true)
    } else if has("light") {
        Some(false)
    } else {
        None
    };
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let app = value("--app").unwrap_or("Argon".into());
    let accent = value("--accent").and_then(|v| {
        let (l, d) = v.split_once('/')?;
        let rgb = |h: &str| {
            let n = u32::from_str_radix(h, 16).ok()?;
            Some(vn_app::consent_window::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
        };
        Some(vn_app::consent_window::Accent { light_theme: rgb(l)?, dark_theme: rgb(d)? })
    });
    vn_app::consent_window::preview(vn_app::consent_window::Preview {
        capability,
        russian: has("ru"),
        dark,
        app,
        owner: has("owner"),
        accent,
        icon_from: value("--icon"),
        through_desktop: has("desktop") || value("--close-owner-after").is_some(),
        close_owner_after: value("--close-owner-after").and_then(|ms| ms.parse().ok()).map(std::time::Duration::from_millis),
        solid: has("solid"),
    });
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("the permission prompt's own window is Windows-only");
}
