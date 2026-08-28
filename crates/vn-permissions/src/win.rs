//! The Windows backend.
//!
//! # What Windows actually does, and what it does not
//!
//! vue-native ships as an **unpackaged** win32 process (no MSIX identity —
//! `GetCurrentPackageFullName` returns `APPMODEL_ERROR_NO_PACKAGE`, 15700).
//! That one fact decides everything below.
//!
//! ## Reading state: the consent store, not WinRT
//!
//! The obvious API is `Windows.Security.Authorization.AppCapabilityAccess.
//! AppCapability::CheckAccess()`. **It is unusable here**, and not in a way that
//! shows up as an error. Probed on this machine (report: `research/reports/m15/
//! permissions.md` §2.3):
//!
//! ```text
//! AppCapability::Create("microphone")?.CheckAccess()              -> Allowed(4)
//! AppCapability::Create("vnProbeNoSuchCapability")?.CheckAccess() -> Allowed(4)
//! ```
//!
//! A capability that does not exist reports *Allowed*. Of course it does: the
//! check is "does this app's manifest declare the capability, and did the user
//! deny it" — and an unpackaged process has no manifest, so nothing denies
//! anything. Building `status()` on it would report `granted` on a machine where
//! the microphone toggle is off, which is the single worst answer this module
//! could give. So the source of truth is the **consent store** in the registry,
//! which is what the Settings UI writes and what the capture stack enforces:
//!
//! ```text
//! HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager
//!      \ConsentStore\<capability>\Value              REG_SZ Allow|Deny
//!      \ConsentStore\<capability>\NonPackaged\Value  REG_SZ Allow|Deny
//! ```
//!
//! The first is the master toggle ("Microphone access"), the second is "Let
//! desktop apps access your microphone". **Both must say Allow.** There is no
//! per-executable decision for an unpackaged app: the per-exe subkeys under
//! `NonPackaged\` exist, but they carry only `LastUsedTimeStart/Stop`,
//! `LastUserAnnotatedLabel` and `PersistedInDatabase` — usage telemetry that
//! drives the "recently accessed" list in Settings. Not one of the 246
//! microphone entries on the probe machine had a `Value`. Packaged apps do get
//! a per-app `Value`; we are not one.
//!
//! ## There is no `prompt` state on Windows
//!
//! Windows never asks an unpackaged process for consent. The toggles are
//! pre-answered, in Settings, before the app runs. So this backend returns only
//! `Granted`, `Denied` and `Unsupported` — never `Prompt`. That is the concrete
//! form of "on Windows you mostly do not need to ask": an app should just try
//! the capture, and use `status()` to explain a failure rather than to gate a
//! first attempt.
//!
//! ## Requesting: open Settings, then watch
//!
//! There is no unpackaged API that raises the consent UI.
//! `AppCapability::RequestAccessAsync` is packaged-only and, given the
//! `CheckAccess` result above, has nothing to ask about here anyway. What does
//! work is the `ms-settings:` deep link — `Launcher::QueryUriSupportAsync`
//! reports `Available(0)` for the scheme, and `NotSupported(3)` for a bogus
//! scheme, so the handler resolution is real. `request()` therefore opens the
//! exact privacy page and then waits for the store to change (below), settling
//! the promise with whatever the user did.
//!
//! ## Change notification is real
//!
//! `RegNotifyChangeKeyValue` with `REG_NOTIFY_THREAD_AGNOSTIC` gives a genuine
//! OS-level subscription — no polling — so `PermissionStatus`'s `change` event
//! is honest here. One watcher thread waits on every consent key at once.
//!
//! It is armed **without** `bWatchSubtree` and with `REG_NOTIFY_CHANGE_LAST_SET`
//! only. That is not a micro-optimisation: with the subtree flag, every write
//! to the per-exe telemetry keys wakes the watcher — and those are written every
//! time *any other application on the machine* touches a microphone. The watcher
//! also re-computes the effective state and drops wakeups that did not change
//! it, so a `change` event always corresponds to a real change.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use vn_runtime::redjs::webapi::permissions::{
    Capability, PermissionHost, PermissionState, RequestOutcome,
};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET,
    REG_NOTIFY_THREAD_AGNOSTIC, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CONSENT: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";
const PUSH_NOTIFICATIONS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\PushNotifications";
/// Documented scope is packaged apps, so it is honoured as a DENY overlay only —
/// never to force `Granted` over a user toggle that says otherwise.
const APP_PRIVACY_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\AppPrivacy";

/// How long `request()` waits for the user to come back from Settings before it
/// settles with whatever the state is by then.
///
/// A promise that can hang forever is worse than a promise that resolves
/// `denied`: the app is left with a spinner and no way to recover. Three minutes
/// is long enough to find the toggle and short enough that a forgotten Settings
/// window does not leak a pending request for the life of the process.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

// ---------------------------------------------------------------------------
// The capability table
// ---------------------------------------------------------------------------

/// The consent-store key name for a capability, if it is gated there at all.
fn consent_key(cap: Capability) -> Option<&'static str> {
    match cap {
        Capability::Microphone => Some("microphone"),
        // "webcam", not "camera" — the store predates the web's spelling.
        Capability::Camera => Some("webcam"),
        // The consent store has two capture keys. `graphicsCaptureProgrammatic`
        // is the one that gates capture started by the app itself, which is what
        // `display-capture` means; `graphicsCaptureWithoutBorder` only controls
        // whether the yellow capture border is drawn.
        Capability::DisplayCapture => Some("graphicsCaptureProgrammatic"),
        Capability::Geolocation => Some("location"),
        Capability::Notifications | Capability::ClipboardRead | Capability::ClipboardWrite => None,
    }
}

/// The `LetAppsAccess*` policy value, where one exists.
fn policy_value(cap: Capability) -> Option<&'static str> {
    match cap {
        Capability::Microphone => Some("LetAppsAccessMicrophone"),
        Capability::Camera => Some("LetAppsAccessCamera"),
        Capability::Geolocation => Some("LetAppsAccessLocation"),
        Capability::Notifications => Some("LetAppsAccessNotifications"),
        _ => None,
    }
}

/// The Settings page that owns this capability's toggle.
///
/// Notifications go to `ms-settings:notifications` rather than a privacy page:
/// the toggle a user needs is the per-app one on the notifications page, and the
/// privacy pages do not have it.
fn settings_uri(cap: Capability) -> Option<&'static str> {
    match cap {
        Capability::Microphone => Some("ms-settings:privacy-microphone"),
        Capability::Camera => Some("ms-settings:privacy-webcam"),
        Capability::DisplayCapture => Some("ms-settings:privacy-graphicscaptureprogrammatic"),
        Capability::Geolocation => Some("ms-settings:privacy-location"),
        Capability::Notifications => Some("ms-settings:notifications"),
        Capability::ClipboardRead | Capability::ClipboardWrite => None,
    }
}

// ---------------------------------------------------------------------------
// Registry reads
// ---------------------------------------------------------------------------

fn read_sz(root: HKEY, path: &str, name: &str) -> Option<String> {
    let path = HSTRING::from(path);
    let name = HSTRING::from(name);
    let mut len: u32 = 0;
    unsafe {
        // Size query first: RegGetValueW reports the byte length including the
        // terminator, and refuses to guess for us. The out-type parameter is
        // `None` because `RRF_RT_REG_SZ` already refuses anything but REG_SZ.
        if RegGetValueW(root, &path, &name, RRF_RT_REG_SZ, None, None, Some(&mut len))
            != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2).max(1)];
        if RegGetValueW(
            root,
            &path,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let chars = (len as usize / 2).min(buf.len());
        let s: Vec<u16> = buf[..chars].iter().copied().take_while(|c| *c != 0).collect();
        Some(String::from_utf16_lossy(&s))
    }
}

fn read_dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
    let path = HSTRING::from(path);
    let name = HSTRING::from(name);
    let mut out: u32 = 0;
    let mut len: u32 = 4;
    unsafe {
        if RegGetValueW(
            root,
            &path,
            &name,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut out as *mut u32).cast()),
            Some(&mut len),
        ) != ERROR_SUCCESS
        {
            return None;
        }
    }
    Some(out)
}

/// One consent-store `Value`, as a tri-state.
///
/// `None` (key or value absent) is **not** a denial. Windows' documented default
/// for every one of these toggles is on, and the keys are created lazily the
/// first time something writes them; reporting `denied` for an absent key would
/// make an app refuse to open a device that would have worked.
fn consent_value(path: &str) -> Option<bool> {
    match read_sz(HKEY_CURRENT_USER, path, "Value")?.as_str() {
        "Allow" => Some(true),
        "Deny" => Some(false),
        _ => None,
    }
}

/// The group-policy overlay. `Some(false)` = force-denied.
///
/// Only the deny direction is honoured: the documented scope of these policies
/// is packaged ("Windows") apps, so treating `ForceAllow(1)` as granting a
/// desktop app access would be an invention. A `ForceDeny(2)` is a strong signal
/// from an administrator and costs nothing to respect.
fn policy_denies(cap: Capability) -> bool {
    let Some(name) = policy_value(cap) else { return false };
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if read_dword(root, APP_PRIVACY_POLICY, name) == Some(2) {
            return true;
        }
    }
    false
}

/// The effective state for THIS process. Pure over the registry, so it is the
/// one function the read-only tests exercise against the real machine.
pub fn read_state(cap: Capability) -> PermissionState {
    if policy_denies(cap) {
        return PermissionState::Denied;
    }
    match cap {
        // Windows has no consent gate on the clipboard at all: `OpenClipboard`
        // is available to any process on the interactive desktop. Reporting
        // anything but `granted` would be a lie about the platform.
        Capability::ClipboardRead | Capability::ClipboardWrite => PermissionState::Granted,

        Capability::Notifications => notifications_state(),

        _ => {
            let Some(key) = consent_key(cap) else { return PermissionState::Unsupported };
            let master = consent_value(&format!(r"{CONSENT}\{key}"));
            let desktop = consent_value(&format!(r"{CONSENT}\{key}\NonPackaged"));
            // Both gates, and absent means "nothing denies it".
            if master == Some(false) || desktop == Some(false) {
                PermissionState::Denied
            } else {
                PermissionState::Granted
            }
        }
    }
}

/// Toast state for an unpackaged app.
///
/// Two switches, no consent store: the global `ToastEnabled` (absent = on), and
/// the per-AppUserModelID `Enabled` under `Notifications\Settings\<AUMID>`
/// (absent = on) that Settings writes when the user turns an app's
/// notifications off.
///
/// This reports what the *user preference* is. It deliberately does not report
/// whether this process can currently raise a toast, which additionally requires
/// a Start-menu shortcut carrying the AUMID — probing an unregistered id gives
/// `ELEMENT_NOT_FOUND (0x80070490)` from `CreateToastNotifierWithId`. That is an
/// installer concern, not a permission, and conflating the two would report
/// `denied` on a dev machine where nothing is denied.
fn notifications_state() -> PermissionState {
    if read_dword(HKEY_CURRENT_USER, PUSH_NOTIFICATIONS, "ToastEnabled") == Some(0) {
        return PermissionState::Denied;
    }
    match aumid() {
        Some(id) => {
            let path =
                format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Notifications\Settings\{id}");
            if read_dword(HKEY_CURRENT_USER, &path, "Enabled") == Some(0) {
                PermissionState::Denied
            } else {
                PermissionState::Granted
            }
        }
        None => PermissionState::Granted,
    }
}

static AUMID: OnceLock<Option<String>> = OnceLock::new();

/// The AppUserModelID this process publishes toasts under, if the host set one.
///
/// Set by the embedder before `install()`; until vue-native registers a
/// Start-menu shortcut there is nothing to set, and the per-app toggle simply
/// does not exist yet.
pub fn set_aumid(id: &str) {
    let _ = AUMID.set(Some(id.to_string()));
}

fn aumid() -> Option<String> {
    AUMID.get().cloned().flatten()
}

// ---------------------------------------------------------------------------
// The host
// ---------------------------------------------------------------------------

struct PendingRequest {
    cap: Capability,
    was: PermissionState,
    started: Instant,
}

#[derive(Default)]
pub struct WindowsHost {
    watcher_started: AtomicBool,
    next_token: AtomicU32,
    pending: Mutex<Vec<(u32, PendingRequest)>>,
}

impl WindowsHost {
    pub const fn new() -> WindowsHost {
        WindowsHost {
            watcher_started: AtomicBool::new(false),
            next_token: AtomicU32::new(0),
            pending: Mutex::new(Vec::new()),
        }
    }

    fn ensure_watcher(&self) {
        if self.watcher_started.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::Builder::new()
            .name("vn-permissions-watch".into())
            .spawn(watch_loop)
            .expect("spawning the permission watcher");
    }
}

impl PermissionHost for WindowsHost {
    fn platform(&self) -> &'static str {
        "Windows"
    }

    fn status(&self, cap: Capability) -> PermissionState {
        read_state(cap)
    }

    fn request(&self, cap: Capability) -> RequestOutcome {
        let now = read_state(cap);
        // Already true, or nothing on this platform could change it: do not open
        // a Settings window at the user for no reason.
        if now == PermissionState::Granted || now == PermissionState::Unsupported {
            return RequestOutcome::Settled(now);
        }
        let Some(uri) = settings_uri(cap) else {
            return RequestOutcome::Settled(now);
        };
        if !open_uri(uri) {
            return RequestOutcome::Settled(now);
        }
        self.ensure_watcher();
        let token = self.next_token.fetch_add(1, Ordering::SeqCst).wrapping_add(1).max(1);
        self.pending.lock().unwrap().push((
            token,
            PendingRequest { cap, was: now, started: Instant::now() },
        ));
        RequestOutcome::Pending(token)
    }

    fn drain_changes(&self) -> Option<Vec<Capability>> {
        self.ensure_watcher();
        Some(std::mem::take(&mut *changes().lock().unwrap()))
    }

    fn drain_settled(&self) -> Vec<(u32, PermissionState)> {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        pending.retain(|(token, p)| {
            let now = read_state(p.cap);
            if now != p.was {
                out.push((*token, now));
                return false;
            }
            if p.started.elapsed() >= REQUEST_TIMEOUT {
                // Settle with the truth rather than leaving an `await` hanging
                // because the user closed Settings without touching anything.
                out.push((*token, now));
                return false;
            }
            true
        });
        out
    }
}

/// `ShellExecuteW` rather than `Launcher::LaunchUriAsync`: no WinRT apartment,
/// no async, and it is the call that works identically packaged or not. The
/// return value is the legacy `> 32 means success` HINSTANCE convention.
fn open_uri(uri: &str) -> bool {
    let uri = HSTRING::from(uri);
    let verb = HSTRING::from("open");
    let rc = unsafe {
        ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(uri.as_ptr()), None, None, SW_SHOWNORMAL)
    };
    rc.0 as usize > 32
}

// ---------------------------------------------------------------------------
// The watcher
// ---------------------------------------------------------------------------

fn changes() -> &'static Mutex<Vec<Capability>> {
    static C: OnceLock<Mutex<Vec<Capability>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// Every registry key whose `Value` participates in an effective state, paired
/// with the capabilities that state belongs to.
fn watched_keys() -> Vec<(String, Vec<Capability>)> {
    let mut out: Vec<(String, Vec<Capability>)> = Vec::new();
    for cap in Capability::ALL.iter().copied() {
        if let Some(key) = consent_key(cap) {
            out.push((format!(r"{CONSENT}\{key}"), vec![cap]));
            out.push((format!(r"{CONSENT}\{key}\NonPackaged"), vec![cap]));
        }
    }
    out.push((PUSH_NOTIFICATIONS.to_string(), vec![Capability::Notifications]));
    out
}

/// Waits on every consent key at once and reports only *effective* changes.
///
/// The diff is the load-bearing part. A wakeup means "something under this key
/// was written", which on a busy machine happens whenever another application
/// starts using a microphone; re-reading and comparing turns that into silence.
fn watch_loop() {
    struct Watch {
        key: HKEY,
        event: HANDLE,
        caps: Vec<Capability>,
    }

    let mut watches: Vec<Watch> = Vec::new();
    unsafe {
        for (path, caps) in watched_keys() {
            let mut key = HKEY::default();
            let wide = HSTRING::from(path.as_str());
            if RegOpenKeyExW(HKEY_CURRENT_USER, &wide, None, KEY_READ | KEY_NOTIFY, &mut key)
                != ERROR_SUCCESS
            {
                // A capability whose key does not exist yet simply has no
                // subscription; `status()` still reads it correctly.
                continue;
            }
            let Ok(event) = CreateEventW(None, true, false, None) else {
                let _ = RegCloseKey(key);
                continue;
            };
            watches.push(Watch { key, event, caps });
        }
    }
    if watches.is_empty() {
        return;
    }

    let mut snapshot: HashMap<Capability, PermissionState> =
        Capability::ALL.iter().map(|c| (*c, read_state(*c))).collect();

    // WaitForMultipleObjects tops out at 64 handles; the table is ~9.
    let handles: Vec<HANDLE> = watches.iter().map(|w| w.event).collect();
    loop {
        unsafe {
            for w in &watches {
                // Re-arm every pass: a notification is one-shot.
                let _ = RegNotifyChangeKeyValue(
                    w.key,
                    // NOT a subtree watch, and LAST_SET only — see the module
                    // header: the per-exe telemetry keys underneath are written
                    // by every other app on the machine.
                    false,
                    REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_THREAD_AGNOSTIC,
                    Some(w.event),
                    true,
                );
            }
            let rc = WaitForMultipleObjects(&handles, false, INFINITE);
            let idx = rc.0 as usize;
            if idx >= watches.len() {
                break;
            }
            let _ = windows::Win32::System::Threading::ResetEvent(watches[idx].event);

            let mut hit: Vec<Capability> = Vec::new();
            for cap in watches[idx].caps.iter().copied() {
                let now = read_state(cap);
                if snapshot.get(&cap) != Some(&now) {
                    snapshot.insert(cap, now);
                    hit.push(cap);
                }
            }
            if !hit.is_empty() {
                let mut q = changes().lock().unwrap();
                for c in hit {
                    if !q.contains(&c) {
                        q.push(c);
                    }
                }
            }
        }
    }

    unsafe {
        for w in watches {
            let _ = CloseHandle(w.event);
            let _ = RegCloseKey(w.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_has_a_deliberate_windows_mapping() {
        // The point is that no capability falls through by accident: each is
        // either consent-store gated, or explicitly not gated, and the ones that
        // are not gated must say why in the match above.
        for cap in Capability::ALL.iter().copied() {
            let gated = consent_key(cap).is_some();
            match cap {
                Capability::Microphone
                | Capability::Camera
                | Capability::DisplayCapture
                | Capability::Geolocation => assert!(gated, "{cap:?} must be consent-store gated"),
                Capability::Notifications
                | Capability::ClipboardRead
                | Capability::ClipboardWrite => {
                    assert!(!gated, "{cap:?} has no consent-store key")
                }
            }
        }
    }

    #[test]
    fn the_consent_key_names_are_the_windows_spellings_not_the_web_ones() {
        // `camera` -> `webcam` and `display-capture` ->
        // `graphicsCaptureProgrammatic` are the two that a refactor "tidies"
        // into non-existent keys, which read as absent, which reads as granted.
        assert_eq!(consent_key(Capability::Camera), Some("webcam"));
        assert_eq!(consent_key(Capability::DisplayCapture), Some("graphicsCaptureProgrammatic"));
        assert_eq!(consent_key(Capability::Microphone), Some("microphone"));
        assert_eq!(consent_key(Capability::Geolocation), Some("location"));
    }

    #[test]
    fn the_clipboard_is_granted_because_windows_has_no_gate_for_it() {
        assert_eq!(read_state(Capability::ClipboardRead), PermissionState::Granted);
        assert_eq!(read_state(Capability::ClipboardWrite), PermissionState::Granted);
    }

    #[test]
    fn reading_the_real_consent_store_never_says_prompt_and_never_panics() {
        // READ-ONLY against the live machine. It cannot assert a particular
        // answer (the operator's privacy settings are not ours to pin), but it
        // does pin the two things that must hold on any Windows box: every
        // capability produces an answer, and `Prompt` is not among them, because
        // Windows never asks an unpackaged process.
        for cap in Capability::ALL.iter().copied() {
            let st = read_state(cap);
            assert_ne!(
                st,
                PermissionState::Prompt,
                "{cap:?}: Windows has no prompt state for an unpackaged app"
            );
        }
    }

    #[test]
    fn a_missing_key_reads_as_granted_rather_than_denied() {
        // The default direction matters: an absent toggle means Windows has not
        // been told to deny anything.
        assert_eq!(consent_value(&format!(r"{CONSENT}\vnProbeNoSuchCapability")), None);
    }

    #[test]
    fn every_requestable_capability_has_a_settings_page() {
        for cap in Capability::ALL.iter().copied() {
            let has_page = settings_uri(cap).is_some();
            let is_clipboard =
                matches!(cap, Capability::ClipboardRead | Capability::ClipboardWrite);
            assert_eq!(
                has_page, !is_clipboard,
                "{cap:?}: only the clipboard has nothing to open"
            );
        }
    }

    #[test]
    fn the_watch_table_covers_both_gates_of_every_gated_capability() {
        // Watching only the master toggle is a real bug that reads as "changes
        // sometimes do not fire": the "Let desktop apps…" switch lives on the
        // NonPackaged subkey and is the one users actually flip.
        let keys = watched_keys();
        for cap in Capability::ALL.iter().copied() {
            let Some(k) = consent_key(cap) else { continue };
            for suffix in ["", r"\NonPackaged"] {
                let want = format!(r"{CONSENT}\{k}{suffix}");
                assert!(keys.iter().any(|(p, c)| *p == want && c.contains(&cap)), "missing {want}");
            }
        }
    }
}
