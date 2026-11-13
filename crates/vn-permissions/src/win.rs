//! The Windows backend.
//!
//! vue-native is an unpackaged win32 process (no MSIX identity), which decides everything below.
//!
//! State is read from the consent store in the registry, not WinRT: `AppCapability::CheckAccess()`
//! reports `Allowed` even for capabilities that do not exist when the caller is unpackaged, so it
//! would say `granted` with the microphone toggle off.
//!
//! ```text
//! HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager
//!      \ConsentStore\<capability>\Value              REG_SZ Allow|Deny
//!      \ConsentStore\<capability>\NonPackaged\Value  REG_SZ Allow|Deny
//! ```
//!
//! The first is the master toggle, the second "Let desktop apps access your microphone"; both
//! must say Allow. There is no per-executable decision for an unpackaged app (the per-exe subkeys
//! carry only usage telemetry).
//!
//! There is no `prompt` state: Windows never asks an unpackaged process, so this backend returns
//! only `Granted`, `Denied` and `Unsupported`.
//!
//! No unpackaged API raises the consent UI, so `request()` opens the `ms-settings:` privacy page
//! and waits for the store to change, settling with whatever the user did. Requests for a
//! capability whose page is already open join it.
//!
//! Changes: `RegNotifyChangeKeyValue` with `REG_NOTIFY_THREAD_AGNOSTIC` is a real subscription
//! (no polling); one watcher thread waits on every consent key. It is armed without
//! `bWatchSubtree` and with `REG_NOTIFY_CHANGE_LAST_SET` only: the per-exe telemetry keys are
//! written whenever any other app touches a microphone. The watcher also drops wakeups that did
//! not change the effective state. Lazily written keys (`webcam\NonPackaged`, the policy keys)
//! are watched through their nearest existing ancestor, then directly once they exist.

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
    HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    REG_NOTIFY_THREAD_AGNOSTIC, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CONSENT: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";
const PUSH_NOTIFICATIONS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\PushNotifications";
/// Honoured as a DENY overlay only: its documented scope is packaged apps, so it never forces
/// `Granted` over a user toggle.
const APP_PRIVACY_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\AppPrivacy";

/// How long `request()` waits for the user to come back from Settings before settling with the
/// state by then; a promise that hangs forever leaves the app with no recovery.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// The consent-store key name for a capability, if it is gated there at all.
fn consent_key(cap: Capability) -> Option<&'static str> {
    match cap {
        Capability::Microphone => Some("microphone"),
        // "webcam", not "camera" — the store predates the web's spelling.
        Capability::Camera => Some("webcam"),
        // `graphicsCaptureProgrammatic` gates app-started capture, i.e. `display-capture`;
        // `graphicsCaptureWithoutBorder` only controls the yellow border.
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

/// The Settings page that owns this capability's toggle. Notifications go to their own page: the
/// per-app toggle is there, not on the privacy pages.
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

fn read_sz(root: HKEY, path: &str, name: &str) -> Option<String> {
    let path = HSTRING::from(path);
    let name = HSTRING::from(name);
    let mut len: u32 = 0;
    unsafe {
        // Size query first: RegGetValueW reports the byte length including the terminator.
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

/// One consent-store `Value` as a tri-state. `None` (absent) is not a denial: the toggles default
/// on and the keys are created lazily.
fn consent_value(path: &str) -> Option<bool> {
    match read_sz(HKEY_CURRENT_USER, path, "Value")?.as_str() {
        "Allow" => Some(true),
        "Deny" => Some(false),
        _ => None,
    }
}

/// The group-policy overlay; `Some(false)` = force-denied. Only the deny direction is honoured:
/// the policies' documented scope is packaged apps.
fn policy_denies(cap: Capability) -> bool {
    let Some(name) = policy_value(cap) else { return false };
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if read_dword(root, APP_PRIVACY_POLICY, name) == Some(2) {
            return true;
        }
    }
    false
}

/// The effective state for this process.
pub fn read_state(cap: Capability) -> PermissionState {
    if policy_denies(cap) {
        return PermissionState::Denied;
    }
    match cap {
        // No consent gate on the clipboard: `OpenClipboard` is open to any process on the desktop.
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

/// Toast state for an unpackaged app: the global `ToastEnabled` and the per-AUMID `Enabled`
/// (both absent = on).
///
/// This is the user preference, not whether the process can raise a toast now (that also needs a
/// Start-menu shortcut carrying the AUMID, an installer concern).
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

/// The AppUserModelID toasts are published under, set by the embedder before `install()`.
pub fn set_aumid(id: &str) {
    let _ = AUMID.set(Some(id.to_string()));
}

fn aumid() -> Option<String> {
    AUMID.get().cloned().flatten()
}

#[derive(Clone, Copy)]
struct PendingRequest {
    cap: Capability,
    was: PermissionState,
    started: Instant,
}

/// Joins `token` to a request already waiting on `cap`'s Settings page or, for the first one,
/// opens the page. `false` when the page could not be opened.
fn join_or_open(
    pending: &Mutex<Vec<(u32, PendingRequest)>>,
    token: u32,
    cap: Capability,
    now: PermissionState,
    open: impl FnOnce() -> bool,
) -> bool {
    let joined = pending.lock().unwrap().iter().find(|(_, p)| p.cap == cap).map(|(_, p)| *p);
    let request = match joined {
        Some(p) => p,
        // Not under the lock: ShellExecute is slow.
        None if open() => PendingRequest { cap, was: now, started: Instant::now() },
        None => return false,
    };
    pending.lock().unwrap().push((token, request));
    true
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
        // Already true, or nothing could change it: do not open Settings for no reason.
        if now == PermissionState::Granted || now == PermissionState::Unsupported {
            return RequestOutcome::Settled(now);
        }
        let Some(uri) = settings_uri(cap) else {
            return RequestOutcome::Settled(now);
        };
        let token = self.next_token.fetch_add(1, Ordering::SeqCst).wrapping_add(1).max(1);
        if !join_or_open(&self.pending, token, cap, now, || open_uri(uri)) {
            return RequestOutcome::Settled(now);
        }
        self.ensure_watcher();
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
                // Settle with the truth, not a hanging `await`, if Settings closed untouched.
                out.push((*token, now));
                return false;
            }
            true
        });
        out
    }
}

/// `ShellExecuteW`: no WinRT apartment or async, same packaged or not. Success is the legacy
/// `> 32` HINSTANCE convention.
fn open_uri(uri: &str) -> bool {
    let uri = HSTRING::from(uri);
    let verb = HSTRING::from("open");
    let rc = unsafe {
        ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(uri.as_ptr()), None, None, SW_SHOWNORMAL)
    };
    rc.0 as usize > 32
}

fn changes() -> &'static Mutex<Vec<Capability>> {
    static C: OnceLock<Mutex<Vec<Capability>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// Every registry key participating in an effective state, with its hive and capabilities: both
/// consent gates, the toast switches and the policy overlay in both hives.
fn watched_keys() -> Vec<(HKEY, String, Vec<Capability>)> {
    let mut out: Vec<(HKEY, String, Vec<Capability>)> = Vec::new();
    for cap in Capability::ALL.iter().copied() {
        if let Some(key) = consent_key(cap) {
            out.push((HKEY_CURRENT_USER, format!(r"{CONSENT}\{key}"), vec![cap]));
            out.push((HKEY_CURRENT_USER, format!(r"{CONSENT}\{key}\NonPackaged"), vec![cap]));
        }
    }
    out.push((HKEY_CURRENT_USER, PUSH_NOTIFICATIONS.to_string(), vec![Capability::Notifications]));
    if let Some(id) = aumid() {
        out.push((
            HKEY_CURRENT_USER,
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Notifications\Settings\{id}"),
            vec![Capability::Notifications],
        ));
    }
    let policed: Vec<Capability> = Capability::ALL.iter().copied().filter(|c| policy_value(*c).is_some()).collect();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        out.push((root, APP_PRIVACY_POLICY.to_string(), policed.clone()));
    }
    out
}

fn open_key(root: HKEY, path: &str) -> Option<HKEY> {
    let mut key = HKEY::default();
    let wide = HSTRING::from(path);
    (unsafe { RegOpenKeyExW(root, &wide, None, KEY_READ | KEY_NOTIFY, &mut key) } == ERROR_SUCCESS).then_some(key)
}

/// One key a state depends on, and the key actually watched for it. Lazily written keys are
/// watched through their nearest existing ancestor (`REG_NOTIFY_CHANGE_NAME`, no subtree), then
/// directly once they exist.
struct Watch {
    root: HKEY,
    path: String,
    key: Option<HKEY>,
    /// How much of `path` the open key is (its length in bytes): all of it,
    /// or an ancestor's.
    open: usize,
    armed: bool,
    event: HANDLE,
}

impl Watch {
    fn new(root: HKEY, path: String) -> Option<Watch> {
        let event = unsafe { CreateEventW(None, true, false, None) }.ok()?;
        let mut w = Watch { root, path, key: None, open: 0, armed: false, event };
        w.reopen();
        Some(w)
    }

    fn exact(&self) -> bool {
        self.key.is_some() && self.open == self.path.len()
    }

    /// Opens the key, or its nearest existing ancestor while it is missing.
    fn reopen(&mut self) {
        if self.exact() {
            return;
        }
        let path = self.path.clone();
        let mut at = path.as_str();
        loop {
            if at.len() <= self.open && self.key.is_some() {
                // Nothing deeper exists yet: keep the ancestor already watched.
                return;
            }
            if let Some(key) = open_key(self.root, at) {
                self.close();
                self.key = Some(key);
                self.open = at.len();
                return;
            }
            match at.rfind('\\') {
                Some(i) => at = &at[..i],
                None => return,
            }
        }
    }

    /// Asks for the next change. A notification is one-shot, so this runs
    /// again after every wakeup.
    fn arm(&mut self) {
        if self.armed {
            return;
        }
        for _ in 0..2 {
            let Some(key) = self.key else { return };
            // Not a subtree watch, LAST_SET only (see the module header); an ancestor is watched
            // for a missing key.
            let filter = if self.exact() { REG_NOTIFY_CHANGE_LAST_SET } else { REG_NOTIFY_CHANGE_NAME };
            let rc = unsafe {
                RegNotifyChangeKeyValue(key, false, filter | REG_NOTIFY_THREAD_AGNOSTIC, Some(self.event), true)
            };
            if rc == ERROR_SUCCESS {
                self.armed = true;
                return;
            }
            // Deleted under us: back to watching for it to come back.
            self.close();
            self.reopen();
        }
    }

    fn close(&mut self) {
        if let Some(key) = self.key.take() {
            unsafe {
                let _ = RegCloseKey(key);
            }
        }
        self.open = 0;
        self.armed = false;
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.close();
        unsafe {
            let _ = CloseHandle(self.event);
        }
    }
}

/// Every watched key, waited on at once.
struct Watcher {
    watches: Vec<Watch>,
}

impl Watcher {
    fn new(keys: impl IntoIterator<Item = (HKEY, String)>) -> Watcher {
        Watcher { watches: keys.into_iter().filter_map(|(root, path)| Watch::new(root, path)).collect() }
    }

    /// Blocks until one of the keys changes (`Some(index)`), or `timeout_ms`
    /// passes or the wait fails (`None`). A key created meanwhile is watched
    /// directly from then on.
    fn wait(&mut self, timeout_ms: u32) -> Option<usize> {
        for w in &mut self.watches {
            w.arm();
        }
        // WaitForMultipleObjects tops out at 64 handles; the table is ~13.
        let handles: Vec<HANDLE> = self.watches.iter().map(|w| w.event).collect();
        if handles.is_empty() {
            return None;
        }
        let rc = unsafe { WaitForMultipleObjects(&handles, false, timeout_ms) };
        let idx = rc.0 as usize;
        if idx >= handles.len() {
            return None;
        }
        let w = &mut self.watches[idx];
        unsafe {
            let _ = ResetEvent(w.event);
        }
        w.armed = false;
        for w in &mut self.watches {
            if !w.exact() {
                let before = (w.key.is_some(), w.open);
                w.reopen();
                if (w.key.is_some(), w.open) != before {
                    w.armed = false;
                }
            }
        }
        Some(idx)
    }
}

/// Waits on every key at once and reports only effective changes: a wakeup often changes
/// nothing, and re-reading every capability (a key watched via its ancestor wakes for the
/// ancestor) filters that.
fn watch_loop() {
    let mut watcher = Watcher::new(watched_keys().into_iter().map(|(root, path, _)| (root, path)));
    let mut snapshot: HashMap<Capability, PermissionState> =
        Capability::ALL.iter().map(|c| (*c, read_state(*c))).collect();
    while watcher.wait(INFINITE).is_some() {
        let mut hit: Vec<Capability> = Vec::new();
        for cap in Capability::ALL.iter().copied() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_has_a_deliberate_windows_mapping() {
        // No capability may fall through: each is consent-store gated or explicitly not, and
        // says why in the match above.
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
        // `camera` -> `webcam` and `display-capture` -> `graphicsCaptureProgrammatic`: a "tidy"
        // into non-existent keys reads as absent, i.e. granted.
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
        // Read-only against the live machine: every capability answers and `Prompt` is never
        // among them.
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
        // Watching only the master toggle misses the "Let desktop apps..." switch on the
        // NonPackaged subkey.
        let keys = watched_keys();
        for cap in Capability::ALL.iter().copied() {
            let Some(k) = consent_key(cap) else { continue };
            for suffix in ["", r"\NonPackaged"] {
                let want = format!(r"{CONSENT}\{k}{suffix}");
                assert!(
                    keys.iter().any(|(r, p, c)| *r == HKEY_CURRENT_USER && *p == want && c.contains(&cap)),
                    "missing {want}"
                );
            }
        }
        // The deny overlay is part of the state too, in both hives.
        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            assert!(keys.iter().any(|(r, p, c)| *r == root && p == APP_PRIVACY_POLICY && c.contains(&Capability::Microphone)));
        }
    }

    /// A key under `HKCU\Software` of this test's own, deleted at the end.
    struct ScratchKey(String);
    impl ScratchKey {
        fn new(name: &str) -> ScratchKey {
            let k = ScratchKey(format!(r"Software\vn-permissions-test-{name}-{}", std::process::id()));
            k.delete();
            k
        }
        fn set(&self, sub: &str, value: &str) {
            use windows::Win32::System::Registry::{
                RegCreateKeyExW, RegSetValueExW, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
            };
            let path = HSTRING::from(format!(r"{}\{sub}", self.0));
            let mut key = HKEY::default();
            unsafe {
                assert_eq!(
                    RegCreateKeyExW(HKEY_CURRENT_USER, &path, None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None),
                    ERROR_SUCCESS
                );
                let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
                let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
                assert_eq!(RegSetValueExW(key, &HSTRING::from("Value"), None, REG_SZ, Some(bytes)), ERROR_SUCCESS);
                let _ = RegCloseKey(key);
            }
        }
        fn delete(&self) {
            let path = HSTRING::from(self.0.as_str());
            unsafe {
                let _ = windows::Win32::System::Registry::RegDeleteTreeW(HKEY_CURRENT_USER, &path);
            }
        }
    }
    impl Drop for ScratchKey {
        fn drop(&mut self) {
            self.delete();
        }
    }

    #[test]
    fn a_key_created_after_the_watch_began_is_watched_from_then_on() {
        // `webcam\NonPackaged` is created the first time Settings writes it.
        // Watching only the keys that existed at start would never see it.
        let base = ScratchKey::new("lazy");
        base.set("present", "Allow");
        let lazy = format!(r"{}\webcam\NonPackaged", base.0);
        let mut w = Watcher::new([(HKEY_CURRENT_USER, lazy.clone())]);
        assert!(!w.watches[0].exact(), "missing: its nearest ancestor is watched");
        assert_eq!(w.watches[0].open, base.0.len());
        assert_eq!(w.wait(50), None, "nothing happened");

        base.set(r"webcam\NonPackaged", "Deny");
        assert_eq!(w.wait(5000), Some(0), "the ancestor saw the key created");
        assert!(w.watches[0].exact(), "and the key itself is watched from now on");
        // Drain a wakeup the creation's own value write may have left.
        let _ = w.wait(100);
        base.set(r"webcam\NonPackaged", "Allow");
        assert_eq!(w.wait(5000), Some(0), "a toggle of the new key wakes the watcher");
        assert_eq!(consent_value(&lazy), Some(true), "the value the watcher woke for");
    }

    #[test]
    fn requests_made_while_settings_is_open_share_one_window_and_one_wait() {
        let pending = Mutex::new(Vec::new());
        let opened = std::cell::Cell::new(0);
        let open = || {
            opened.set(opened.get() + 1);
            true
        };
        let cap = Capability::Camera;
        assert!(join_or_open(&pending, 1, cap, PermissionState::Denied, open));
        assert!(join_or_open(&pending, 2, cap, PermissionState::Denied, open));
        assert_eq!(opened.get(), 1, "Settings is opened once");
        assert!(join_or_open(&pending, 3, Capability::Microphone, PermissionState::Denied, open));
        assert_eq!(opened.get(), 2, "another capability has a page of its own");
        let p = pending.lock().unwrap();
        assert_eq!(p[0].1.started, p[1].1.started, "one wait: the two settle together");
        drop(p);
        let failed = Mutex::new(Vec::new());
        assert!(!join_or_open(&failed, 4, cap, PermissionState::Denied, || false));
        assert!(failed.lock().unwrap().is_empty(), "nothing waits on a page that never opened");
    }
}
