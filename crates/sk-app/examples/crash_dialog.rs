//! The crash dialog's window on its own, for a look at it (src/crash_window.rs). Nothing crashes:
//! the buttons print what they would do (Copy really copies). Windows only.
//!
//! ```text
//! cargo run -p sk-app --example crash_dialog -- [--lang de] [light|dark] [owner] [expanded] [embedded] [solid]
//!     [--app Name] [--version 1.4.2] [--reason "text"] [--accent 005FB8/60CDFF] [--icon C:\Windows\notepad.exe]
//! ```

#[cfg(target_os = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let dark = if has("dark") {
        Some(true)
    } else if has("light") {
        Some(false)
    } else {
        None
    };
    let accent = value("--accent").and_then(|v| {
        let (l, d) = v.split_once('/')?;
        let rgb = |h: &str| {
            let n = u32::from_str_radix(h, 16).ok()?;
            Some(sk_app::consent_window::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
        };
        Some(sk_app::consent_window::Accent { light_theme: rgb(l)?, dark_theme: rgb(d)? })
    });
    sk_app::crash_window::preview(sk_app::crash_window::Preview {
        lang: value("--lang").unwrap_or_default(),
        dark,
        app: value("--app").unwrap_or("Argon".into()),
        version: Some(value("--version").unwrap_or("1.4.2".into())),
        reason: value("--reason").unwrap_or(
            "already mutably borrowed: BorrowError at crates/sk-tree/src/dispatch.rs:1184:37".into(),
        ),
        owner: has("owner"),
        embedded: has("embedded"),
        expanded: has("expanded"),
        accent,
        icon_from: value("--icon"),
        solid: has("solid"),
    });
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("the crash dialog's own window is Windows-only; elsewhere it is the platform's alert");
}
