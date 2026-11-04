//! vn-clipboard — the platform half of `navigator.clipboard`.
//!
//! # What is where, and why
//!
//! The browser-shaped surface (`navigator.clipboard`, the `Clipboard` class,
//! the promises, the `NotAllowedError` rejections, the inert
//! `document.execCommand`) lives in **redjs**
//! (`redjs::webapi::clipboard`), because it is browser semantics. So does the
//! [`ClipboardHost`] trait it reads through — a trait redjs consumes has to be
//! declared by redjs.
//!
//! This crate is the other half, and it is deliberately tiny: gpui's
//! `App::read_from_clipboard` / `write_to_clipboard`, reached through the
//! commit-phase bridge, wearing that trait.
//!
//! ```text
//!   apps/*  ── await navigator.clipboard.readText() ──┐
//!                                                     │  tier 1: W3C, no extension tier
//!   redjs::webapi::clipboard ── dyn ClipboardHost ────┘
//!                                       ▲
//!                            vn_clipboard::GpuiClipboard
//!                                       │
//!                          gpui::App::{read,write}_…_clipboard
//!                                       │
//!               the same OS clipboard the native <input> uses for Ctrl+C
//! ```
//!
//! # Why gpui's clipboard and not a Win32 one
//!
//! Because there is exactly one OS clipboard and `OpenClipboard` is exclusive.
//! `crates/vn-elements`' text input already copies and pastes through gpui
//! (`text_input/state.rs`), so a second, private Win32 path in this crate would
//! be two owners racing for one handle — and on the frame where they collided,
//! one of them would silently do nothing. Reusing the seam also means the
//! platform quirks (retry on a busy clipboard, CF_UNICODETEXT vs CF_TEXT, the
//! trailing NUL) are solved once, in the place that already had to solve them.
//!
//! # Items: the one place this crate opens the clipboard itself
//!
//! `navigator.clipboard.write([new ClipboardItem({...})])` and `read()` go to
//! the Windows clipboard directly (`win.rs`), because gpui's `ClipboardItem`
//! has no HTML entry and writes an image as its PNG alone — and the apps that
//! paste only bitmaps (Paint, Word, most of what the owner pastes a picture
//! into) need the `CF_DIBV5` / `CF_DIB` Chrome writes beside it. That is not
//! the race the section above is about: both paths open and close the
//! clipboard around one call, on the one thread JS and gpui's text input share,
//! so they are never inside it at the same time.
//!
//! # There is no tier-2 extension here
//!
//! Unlike windowing and permissions, the clipboard needs no `appClipboard`:
//! `navigator.clipboard` with `ClipboardItem` says everything a desktop app
//! wants to say.
//!
//! # Frame phase
//!
//! gpui's clipboard needs an `&App`, which this process only has inside a
//! published frame scope (`vn_runtime::bridge`). Every realistic call site — an
//! event handler, a `devtools` eval, a Vue effect — is inside one. A call from
//! outside (a bare `setTimeout` that somehow escaped the pump, a native thread)
//! rejects with a message that says so, rather than reaching through a null
//! pointer.

use gpui::ClipboardItem;
use vn_runtime::redjs::webapi::clipboard::{self as api, ClipboardHost};
use vn_runtime::with_app;

/// The backend. Stateless: gpui holds the clipboard, this holds nothing.
pub struct GpuiClipboard;

/// The message every out-of-frame call gets. One constant so the two verbs
/// cannot drift apart.
const NO_FRAME: &str = "the clipboard is only reachable from inside a frame (this call ran \
                        outside the gpui publication — see crates/vn-runtime/src/bridge.rs)";

impl ClipboardHost for GpuiClipboard {
    fn platform(&self) -> &'static str {
        if cfg!(target_os = "windows") {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "this platform"
        }
    }

    fn read_text(&self) -> Result<String, String> {
        with_app(|cx| {
            // Two distinct "no text" cases, one answer, on purpose: an empty
            // clipboard and a clipboard holding only an image both read as ""
            // in a browser, because `readText` promises text and there is none.
            // Neither is an error — an app that pastes "" is behaving exactly
            // as it would in Chrome.
            cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default()
        })
        .ok_or_else(|| NO_FRAME.to_string())
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        with_app(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())))
            .ok_or_else(|| NO_FRAME.to_string())
    }

    fn item_types(&self) -> &'static [&'static str] {
        #[cfg(windows)]
        return win::TYPES;
        #[cfg(not(windows))]
        return &[];
    }

    fn write_items(&self, items: &[(String, Vec<u8>)]) -> Result<(), String> {
        #[cfg(windows)]
        return win::write(items);
        #[cfg(not(windows))]
        {
            let _ = items;
            Err(format!("{} has no item clipboard in this build", self.platform()))
        }
    }

    fn read_items(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        #[cfg(windows)]
        return win::read();
        #[cfg(not(windows))]
        {
            let text = self.read_text()?;
            Ok(if text.is_empty() { Vec::new() } else { vec![("text/plain".to_string(), text.into_bytes())] })
        }
    }
}

#[cfg(windows)]
mod win;

/// Installs the backend into redjs.
///
/// Called from `vn-app`'s boot beside `vn_permissions::install`, and NOT
/// dev-gated: copying a room id out of an app is application work, not
/// automation.
///
/// No `&JsRuntime` argument and no `pump()`, unlike every other install in that
/// boot block — and both absences are the point. This crate registers no host
/// functions (there is no `appClipboard` tier), and the clipboard is
/// synchronous, so a promise is already settled by the time `readText()`
/// returns. Nothing to schedule, nothing to drain.
pub fn install() {
    static H: GpuiClipboard = GpuiClipboard;
    api::set_host(&H);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_names_the_platform_it_is_standing_on() {
        // Advisories in redjs quote this; "this platform" would be a bug on a
        // build that actually knows, and it must agree with the name
        // `vn-permissions`' backend uses in ITS advisories about
        // clipboard-read / clipboard-write.
        let expected = if cfg!(target_os = "windows") {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "this platform"
        };
        assert_eq!(GpuiClipboard.platform(), expected);
    }

    #[test]
    fn outside_a_frame_both_verbs_refuse_with_the_same_explanation() {
        // A `cargo test` thread is never inside a gpui publication, so this is
        // the out-of-frame path exactly as an app would hit it — and the
        // property that matters is that it FAILS, loudly, rather than reading
        // through a null `&mut App`.
        let e = GpuiClipboard.read_text().unwrap_err();
        assert!(e.contains("inside a frame"), "{e}");
        assert_eq!(GpuiClipboard.write_text("x").unwrap_err(), e);
    }

    #[test]
    fn install_is_idempotent_and_leaves_a_host_behind() {
        // `set_host` is last-writer-wins, and a boot that ran install twice (a
        // second window, a re-init after a reload) must end up with a host, not
        // with none.
        assert!(api::host().is_none(), "nothing installed a host before this test");
        install();
        install();
        let h = api::host().expect("a host is installed");
        assert_eq!(h.platform(), GpuiClipboard.platform());
    }
}
