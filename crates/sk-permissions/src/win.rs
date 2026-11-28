//! The Windows backend, for an unpackaged win32 process or a full-trust MSIX one.
//!
//! State comes from the registry consent store, which is what the Settings switches write:
//! `AppCapability::CheckAccess()` reports `Allowed` for unpackaged callers even with the microphone
//! toggle off. `HKCU\...\CapabilityAccessManager\ConsentStore\<cap>\Value` is the master toggle;
//! the app's own is `...\NonPackaged\Value` (all desktop apps) or, with package identity,
//! `...\<package family name>\Value` ([`consent_state`]).
//!
//! Unpackaged there is no `prompt`: Windows never asks a desktop app. `request()` opens the
//! `ms-settings:` page and settles with whatever the user did. A packaged app's key says `Prompt`
//! (written at install for a declared capability, at first use otherwise) until the user answers
//! Windows' own question, which `request()` raises through `AppCapability::RequestAccessAsync`.
//!
//! One watcher thread waits (`RegNotifyChangeKeyValue`, thread-agnostic) on every consent key, armed
//! with `REG_NOTIFY_CHANGE_LAST_SET` and no subtree: the per-exe telemetry keys are written whenever
//! any app touches a microphone. Lazily written keys are watched via their nearest existing ancestor.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use sk_runtime::redjs::webapi::permissions::{
    Capability, PermissionHost, PermissionState, RequestOutcome,
};
use sk_windows::core::HSTRING;
use sk_windows::runtime::{AppCapability, AppCapabilityAccessStatus};

use std::ptr::{null, null_mut};

use sk_windows::raw::ShellExecuteW;
use sk_windows::raw::SW_SHOWNORMAL;
use sk_windows::raw::{CloseHandle, ERROR_SUCCESS, HANDLE};
use sk_windows::raw::{CreateEventW, ResetEvent, WaitForMultipleObjects, INFINITE};
use sk_windows::raw::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
    REG_NOTIFY_THREAD_AGNOSTIC, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

const CONSENT: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";
const PUSH_NOTIFICATIONS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\PushNotifications";
const TOAST_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\PushNotifications";
/// Honoured as a DENY overlay only: its documented scope is packaged apps, so it never forces
/// `Granted` over a user toggle.
const APP_PRIVACY_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows\AppPrivacy";

/// How long `request()` waits for the user to come back from Settings; a promise that hangs
/// forever leaves the app with no recovery.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

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

/// The consent-store key of this app's own toggle for the capability `key`.
fn app_consent_path(key: &str, family: Option<&str>) -> String {
    format!(r"{CONSENT}\{key}\{}", family.unwrap_or("NonPackaged"))
}

fn policy_value(cap: Capability) -> Option<&'static str> {
    match cap {
        Capability::Microphone => Some("LetAppsAccessMicrophone"),
        Capability::Camera => Some("LetAppsAccessCamera"),
        Capability::Geolocation => Some("LetAppsAccessLocation"),
        _ => None,
    }
}

/// Notifications have their own Settings page; the per-app toggle is not on the privacy pages.
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
    let path = wide(path);
    let name = wide(name);
    let mut len: u32 = 0;
    unsafe {
        if RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut len,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2).max(1)];
        if RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let chars = (len as usize / 2).min(buf.len());
        let s: Vec<u16> = buf[..chars]
            .iter()
            .copied()
            .take_while(|c| *c != 0)
            .collect();
        Some(String::from_utf16_lossy(&s))
    }
}

fn read_dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
    let path = wide(path);
    let name = wide(name);
    let mut out: u32 = 0;
    let mut len: u32 = 4;
    unsafe {
        if RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut out as *mut u32).cast(),
            &mut len,
        ) != ERROR_SUCCESS
        {
            return None;
        }
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Consent {
    Allow,
    Deny,
    Prompt,
    /// No key or no `Value`: keys are created lazily.
    Unset,
}

fn consent_value(path: &str) -> Consent {
    match read_sz(HKEY_CURRENT_USER, path, "Value").as_deref() {
        Some("Allow") => Consent::Allow,
        Some("Deny") => Consent::Deny,
        Some("Prompt") => Consent::Prompt,
        _ => Consent::Unset,
    }
}

/// `master` is the capability's own switch, `app` this app's. An unpackaged app is never asked, so
/// only a Deny refuses it. A packaged app is granted only by its own Allow: Windows asks it on
/// first use both while its key says `Prompt` and before the key exists (an undeclared capability).
fn consent_state(master: Consent, app: Consent, packaged: bool) -> PermissionState {
    if master == Consent::Deny || app == Consent::Deny {
        PermissionState::Denied
    } else if !packaged || app == Consent::Allow {
        PermissionState::Granted
    } else {
        PermissionState::Prompt
    }
}

/// The group-policy overlay; only the deny direction is honoured (documented scope: packaged apps).
fn policy_denies(cap: Capability) -> bool {
    let Some(name) = policy_value(cap) else {
        return false;
    };
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if read_dword(root, APP_PRIVACY_POLICY, name) == Some(2) {
            return true;
        }
    }
    false
}

pub fn read_state(cap: Capability) -> PermissionState {
    if policy_denies(cap) {
        return PermissionState::Denied;
    }
    match cap {
        // No consent gate on the clipboard: `OpenClipboard` is open to any process on the desktop.
        Capability::ClipboardRead | Capability::ClipboardWrite => PermissionState::Granted,

        Capability::Notifications => notifications_state(),

        _ => {
            let Some(key) = consent_key(cap) else {
                return PermissionState::Unsupported;
            };
            let family = crate::package::family_name();
            consent_state(
                consent_value(&format!(r"{CONSENT}\{key}")),
                consent_value(&app_consent_path(key, family)),
                family.is_some(),
            )
        }
    }
}

/// User or device policy and the global and per-AUMID switches can each block posting.
fn notifications_state() -> PermissionState {
    // WPN.admx "Turn off toast notifications" applies to both user and device.
    // The AppPrivacy notification switch governs reading other apps' notifications.
    if [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .any(|root| read_dword(root, TOAST_POLICY, "NoToastApplicationNotification") == Some(1))
        || read_dword(HKEY_CURRENT_USER, PUSH_NOTIFICATIONS, "ToastEnabled") == Some(0)
    {
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

pub fn set_aumid(id: &str) {
    let _ = AUMID.set(Some(id.to_string()));
}

/// A packaged process's toasts are keyed by the AUMID Windows assigned it.
fn aumid() -> Option<String> {
    AUMID
        .get()
        .cloned()
        .flatten()
        .or_else(|| crate::package::app_user_model_id().map(str::to_owned))
}

const UNANSWERED: i32 = -1;
const NO_ANSWER: i32 = -2;

/// What a request waits on: the user in Settings, or the answer to Windows' own question.
#[derive(Clone)]
enum Wait {
    Settings,
    Question(Arc<AtomicI32>),
}

#[derive(Clone)]
struct PendingRequest {
    cap: Capability,
    was: PermissionState,
    started: Instant,
    wait: Wait,
}

fn join_or_open(
    pending: &Mutex<Vec<(u32, PendingRequest)>>,
    token: u32,
    cap: Capability,
    now: PermissionState,
    open: impl FnOnce() -> Option<Wait>,
) -> bool {
    let joined = pending
        .lock()
        .unwrap()
        .iter()
        .find(|(_, p)| p.cap == cap)
        .map(|(_, p)| p.clone());
    let request = match joined {
        Some(p) => p,
        // Not under the lock: ShellExecute is slow.
        None => match open() {
            Some(wait) => PendingRequest {
                cap,
                was: now,
                started: Instant::now(),
                wait,
            },
            None => return false,
        },
    };
    pending.lock().unwrap().push((token, request));
    true
}

/// What a request settles with once Windows answered its question. An undeclared capability is
/// not Windows' to answer here; it asks when the device opens, so the capture goes ahead.
fn answered_state(answer: AppCapabilityAccessStatus, now: PermissionState) -> PermissionState {
    match answer {
        AppCapabilityAccessStatus::Allowed => PermissionState::Granted,
        AppCapabilityAccessStatus::DeniedByUser | AppCapabilityAccessStatus::DeniedBySystem => {
            PermissionState::Denied
        }
        AppCapabilityAccessStatus::NotDeclaredByApp if now == PermissionState::Prompt => {
            PermissionState::Granted
        }
        _ => now,
    }
}

/// `RequestAccessAsync` must be called on the UI thread, which is the VM's; any other caller gets
/// `None` and the Settings page.
fn ask_windows(cap: Capability) -> Option<Wait> {
    let key = consent_key(cap)?;
    if !on_ui_thread() {
        return None;
    }
    let capability = AppCapability::Create(&HSTRING::from(key)).ok()?;
    let op = capability.RequestAccessAsync().ok()?;
    let answer = Arc::new(AtomicI32::new(UNANSWERED));
    let to = Arc::clone(&answer);
    op.when(move |result| {
        let _alive = &capability;
        to.store(result.map_or(NO_ANSWER, |s| s.0), Ordering::SeqCst);
        sk_runtime::frames::wake();
    })
    .ok()?;
    Some(Wait::Question(answer))
}

fn on_ui_thread() -> bool {
    use sk_windows::raw::{CoGetApartmentType, APTTYPE_MAINSTA, APTTYPE_STA};
    let (mut kind, mut qualifier) = (0, 0);
    // SAFETY: two out-pointers to locals.
    let ok = unsafe { CoGetApartmentType(&mut kind, &mut qualifier) } == 0;
    ok && (kind == APTTYPE_STA || kind == APTTYPE_MAINSTA)
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
            .name("sk-permissions-watch".into())
            .spawn(watch_loop)
            .expect("spawning the permission watcher");
    }
}

impl PermissionHost for WindowsHost {
    fn platform(&self) -> &'static str {
        "Windows"
    }

    /// Packaged, Windows asks for the gated capabilities itself; the app's own prompt stays out.
    fn has_native_prompt(&self, cap: Capability) -> bool {
        crate::package::has_identity() && consent_key(cap).is_some()
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
        let uri = settings_uri(cap);
        let ask = now == PermissionState::Prompt;
        if !ask && uri.is_none() {
            return RequestOutcome::Settled(now);
        }
        let token = self
            .next_token
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1)
            .max(1);
        let open = || {
            ask.then(|| ask_windows(cap))
                .flatten()
                .or_else(|| uri.filter(|u| open_uri(u)).map(|_| Wait::Settings))
        };
        if !join_or_open(&self.pending, token, cap, now, open) {
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
            let settled = match &p.wait {
                Wait::Settings => (now != p.was).then_some(now),
                Wait::Question(answer) => match answer.load(Ordering::SeqCst) {
                    UNANSWERED => None,
                    NO_ANSWER => Some(now),
                    status => {
                        let status = AppCapabilityAccessStatus(status);
                        if status == AppCapabilityAccessStatus::NotDeclaredByApp {
                            undeclared(p.cap);
                        }
                        Some(answered_state(status, now))
                    }
                },
            };
            // Settle with the truth, not a hanging `await`, if nobody answered.
            match settled.or_else(|| (p.started.elapsed() >= REQUEST_TIMEOUT).then_some(now)) {
                Some(state) => {
                    out.push((*token, state));
                    false
                }
                None => true,
            }
        });
        out
    }
}

fn undeclared(cap: Capability) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(key) = consent_key(cap) else { return };
    ONCE.call_once(|| {
        eprintln!(
            "[permissions] the package does not declare \"{key}\": Windows asks only when the device \
             opens. Add \"{key}\" to sk.config.json \"package.windows.msix.capabilities\" so the app \
             can ask first and has its switch in Settings from installation."
        )
    });
}

/// `ShellExecuteW`; success is the legacy `> 32` HINSTANCE convention.
fn open_uri(uri: &str) -> bool {
    let uri = wide(uri);
    let verb = wide("open");
    let rc = unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            uri.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    rc as usize > 32
}

fn changes() -> &'static Mutex<Vec<Capability>> {
    static C: OnceLock<Mutex<Vec<Capability>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

fn watched_keys() -> Vec<(HKEY, String, Vec<Capability>)> {
    let mut out: Vec<(HKEY, String, Vec<Capability>)> = Vec::new();
    for cap in Capability::ALL.iter().copied() {
        if let Some(key) = consent_key(cap) {
            out.push((HKEY_CURRENT_USER, format!(r"{CONSENT}\{key}"), vec![cap]));
            out.push((
                HKEY_CURRENT_USER,
                app_consent_path(key, crate::package::family_name()),
                vec![cap],
            ));
        }
    }
    out.push((
        HKEY_CURRENT_USER,
        PUSH_NOTIFICATIONS.to_string(),
        vec![Capability::Notifications],
    ));
    if let Some(id) = aumid() {
        out.push((
            HKEY_CURRENT_USER,
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Notifications\Settings\{id}"),
            vec![Capability::Notifications],
        ));
    }
    let policed: Vec<Capability> = Capability::ALL
        .iter()
        .copied()
        .filter(|c| policy_value(*c).is_some())
        .collect();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        out.push((root, APP_PRIVACY_POLICY.to_string(), policed.clone()));
        out.push((
            root,
            TOAST_POLICY.to_string(),
            vec![Capability::Notifications],
        ));
    }
    out
}

fn open_key(root: HKEY, path: &str) -> Option<HKEY> {
    let mut key: HKEY = null_mut();
    let path = wide(path);
    (unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | KEY_NOTIFY, &mut key) }
        == ERROR_SUCCESS)
        .then_some(key)
}

/// One key a state depends on, and the key actually watched for it (see the module header).
struct Watch {
    root: HKEY,
    path: String,
    key: Option<HKEY>,
    open: usize,
    armed: bool,
    event: HANDLE,
}

impl Watch {
    fn new(root: HKEY, path: String) -> Option<Watch> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return None;
        }
        let mut w = Watch {
            root,
            path,
            key: None,
            open: 0,
            armed: false,
            event,
        };
        w.reopen();
        Some(w)
    }

    fn exact(&self) -> bool {
        self.key.is_some() && self.open == self.path.len()
    }

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

    /// Asks for the next change; notifications are one-shot, so this re-arms after every wakeup.
    fn arm(&mut self) {
        if self.armed {
            return;
        }
        for _ in 0..2 {
            let Some(key) = self.key else { return };
            // Not a subtree watch, LAST_SET only (see the module header); an ancestor is watched
            // for a missing key.
            let filter = if self.exact() {
                REG_NOTIFY_CHANGE_LAST_SET
            } else {
                REG_NOTIFY_CHANGE_NAME
            };
            let rc = unsafe {
                RegNotifyChangeKeyValue(key, 0, filter | REG_NOTIFY_THREAD_AGNOSTIC, self.event, 1)
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
                RegCloseKey(key);
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
            CloseHandle(self.event);
        }
    }
}

struct Watcher {
    watches: Vec<Watch>,
}

impl Watcher {
    fn new(keys: impl IntoIterator<Item = (HKEY, String)>) -> Watcher {
        Watcher {
            watches: keys
                .into_iter()
                .filter_map(|(root, path)| Watch::new(root, path))
                .collect(),
        }
    }

    /// Blocks until a key changes (`Some(index)`) or the timeout/failure (`None`).
    fn wait(&mut self, timeout_ms: u32) -> Option<usize> {
        for w in &mut self.watches {
            w.arm();
        }
        // WaitForMultipleObjects tops out at 64 handles; the table is ~13.
        let handles: Vec<HANDLE> = self.watches.iter().map(|w| w.event).collect();
        if handles.is_empty() {
            return None;
        }
        let rc = unsafe {
            WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, timeout_ms)
        };
        let idx = rc as usize;
        if idx >= handles.len() {
            return None;
        }
        let w = &mut self.watches[idx];
        unsafe {
            ResetEvent(w.event);
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

/// Reports only effective changes: re-reads every capability, since a wakeup often changes nothing.
fn watch_loop() {
    let mut watcher = Watcher::new(
        watched_keys()
            .into_iter()
            .map(|(root, path, _)| (root, path)),
    );
    let mut snapshot: HashMap<Capability, PermissionState> = Capability::ALL
        .iter()
        .map(|c| (*c, read_state(*c)))
        .collect();
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
    fn the_consent_key_names_are_the_windows_spellings_not_the_web_ones() {
        // `camera` -> `webcam` and `display-capture` -> `graphicsCaptureProgrammatic`: a "tidy"
        // into non-existent keys reads as absent, i.e. granted.
        assert_eq!(consent_key(Capability::Camera), Some("webcam"));
        assert_eq!(
            consent_key(Capability::DisplayCapture),
            Some("graphicsCaptureProgrammatic")
        );
        assert_eq!(consent_key(Capability::Microphone), Some("microphone"));
        assert_eq!(consent_key(Capability::Geolocation), Some("location"));
    }

    #[test]
    fn a_packaged_app_reads_its_family_key_and_an_unpackaged_one_the_desktop_key() {
        assert_eq!(
            app_consent_path("microphone", None),
            format!(r"{CONSENT}\microphone\NonPackaged")
        );
        assert_eq!(
            app_consent_path("webcam", Some("Example.Notes_8wekyb3d8bbwe")),
            format!(r"{CONSENT}\webcam\Example.Notes_8wekyb3d8bbwe")
        );
    }

    #[test]
    fn every_consent_value_windows_writes_is_told_apart() {
        let base = ScratchKey::new("values");
        base.set("allow", "Allow");
        base.set("deny", "Deny");
        base.set("prompt", "Prompt");
        let at = |sub: &str| consent_value(&format!(r"{}\{sub}", base.0));
        assert_eq!(at("allow"), Consent::Allow);
        assert_eq!(at("deny"), Consent::Deny);
        assert_eq!(at("prompt"), Consent::Prompt);
        assert_eq!(at("missing"), Consent::Unset);
    }

    #[test]
    fn an_unpackaged_app_is_refused_only_by_a_deny() {
        use Consent::*;
        for app in [Allow, Prompt, Unset] {
            for master in [Allow, Prompt, Unset] {
                assert_eq!(
                    consent_state(master, app, false),
                    PermissionState::Granted,
                    "{master:?}/{app:?}"
                );
            }
            assert_eq!(consent_state(Deny, app, false), PermissionState::Denied);
        }
        assert_eq!(consent_state(Allow, Deny, false), PermissionState::Denied);
    }

    #[test]
    fn a_packaged_app_is_granted_only_by_its_own_allow() {
        use Consent::{Allow, Deny, Unset};
        use PermissionState::{Denied, Granted};
        assert_eq!(consent_state(Allow, Allow, true), Granted);
        assert_eq!(consent_state(Unset, Allow, true), Granted);
        assert_eq!(
            consent_state(Allow, Consent::Prompt, true),
            PermissionState::Prompt,
            "a declared capability before Windows has asked"
        );
        assert_eq!(
            consent_state(Allow, Unset, true),
            PermissionState::Prompt,
            "an undeclared one: Windows asks on first use"
        );
        assert_eq!(consent_state(Allow, Deny, true), Denied);
        assert_eq!(
            consent_state(Deny, Allow, true),
            Denied,
            "the master switch"
        );
        assert_eq!(consent_state(Deny, Consent::Prompt, true), Denied);
    }

    #[test]
    fn windows_answer_settles_the_request() {
        use AppCapabilityAccessStatus as S;
        use PermissionState::*;
        assert_eq!(answered_state(S::Allowed, Prompt), Granted);
        assert_eq!(answered_state(S::DeniedByUser, Prompt), Denied);
        assert_eq!(answered_state(S::DeniedBySystem, Prompt), Denied);
        assert_eq!(
            answered_state(S::NotDeclaredByApp, Prompt),
            Granted,
            "the capture goes ahead and Windows asks then"
        );
        assert_eq!(answered_state(S::NotDeclaredByApp, Denied), Denied);
        assert_eq!(answered_state(S::UserPromptRequired, Prompt), Prompt);
    }

    #[test]
    fn this_unpackaged_test_process_never_reads_prompt() {
        assert!(!crate::package::has_identity());
        for cap in Capability::ALL.iter().copied() {
            assert_ne!(read_state(cap), PermissionState::Prompt, "{cap:?}");
        }
    }

    #[test]
    fn the_watch_table_covers_both_gates_of_every_gated_capability() {
        let keys = watched_keys();
        for cap in Capability::ALL.iter().copied() {
            let Some(k) = consent_key(cap) else { continue };
            for want in [format!(r"{CONSENT}\{k}"), app_consent_path(k, None)] {
                assert!(
                    keys.iter()
                        .any(|(r, p, c)| *r == HKEY_CURRENT_USER && *p == want && c.contains(&cap)),
                    "missing {want}"
                );
            }
        }
        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            assert!(keys.iter().any(|(r, p, c)| *r == root
                && p == APP_PRIVACY_POLICY
                && c.contains(&Capability::Microphone)));
        }
    }

    struct ScratchKey(String);
    impl ScratchKey {
        fn new(name: &str) -> ScratchKey {
            let k = ScratchKey(format!(
                r"Software\sk-permissions-test-{name}-{}",
                std::process::id()
            ));
            k.delete();
            k
        }
        fn set(&self, sub: &str, value: &str) {
            use sk_windows::raw::{
                RegCreateKeyExW, RegSetValueExW, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
            };
            let path = wide(&format!(r"{}\{sub}", self.0));
            let mut key: HKEY = null_mut();
            unsafe {
                assert_eq!(
                    RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        path.as_ptr(),
                        0,
                        null(),
                        REG_OPTION_NON_VOLATILE,
                        KEY_SET_VALUE,
                        null(),
                        &mut key,
                        null_mut()
                    ),
                    ERROR_SUCCESS
                );
                let value = wide(value);
                assert_eq!(
                    RegSetValueExW(
                        key,
                        wide("Value").as_ptr(),
                        0,
                        REG_SZ,
                        value.as_ptr().cast(),
                        (value.len() * 2) as u32
                    ),
                    ERROR_SUCCESS
                );
                RegCloseKey(key);
            }
        }
        fn delete(&self) {
            let path = wide(&self.0);
            unsafe {
                sk_windows::raw::RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr());
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
        let base = ScratchKey::new("lazy");
        base.set("present", "Allow");
        let lazy = format!(r"{}\webcam\NonPackaged", base.0);
        let mut w = Watcher::new([(HKEY_CURRENT_USER, lazy.clone())]);
        assert!(
            !w.watches[0].exact(),
            "missing: its nearest ancestor is watched"
        );
        assert_eq!(w.watches[0].open, base.0.len());
        assert_eq!(w.wait(50), None, "nothing happened");

        base.set(r"webcam\NonPackaged", "Deny");
        assert_eq!(w.wait(5000), Some(0), "the ancestor saw the key created");
        assert!(
            w.watches[0].exact(),
            "and the key itself is watched from now on"
        );
        let _ = w.wait(100);
        base.set(r"webcam\NonPackaged", "Allow");
        assert_eq!(
            w.wait(5000),
            Some(0),
            "a toggle of the new key wakes the watcher"
        );
        assert_eq!(
            consent_value(&lazy),
            Consent::Allow,
            "the value the watcher woke for"
        );
    }

    #[test]
    fn requests_made_while_settings_is_open_share_one_window_and_one_wait() {
        let pending = Mutex::new(Vec::new());
        let opened = std::cell::Cell::new(0);
        let open = || {
            opened.set(opened.get() + 1);
            Some(Wait::Settings)
        };
        let cap = Capability::Camera;
        assert!(join_or_open(
            &pending,
            1,
            cap,
            PermissionState::Denied,
            open
        ));
        assert!(join_or_open(
            &pending,
            2,
            cap,
            PermissionState::Denied,
            open
        ));
        assert_eq!(opened.get(), 1, "Settings is opened once");
        assert!(join_or_open(
            &pending,
            3,
            Capability::Microphone,
            PermissionState::Denied,
            open
        ));
        assert_eq!(opened.get(), 2, "another capability has a page of its own");
        let p = pending.lock().unwrap();
        assert_eq!(
            p[0].1.started, p[1].1.started,
            "one wait: the two settle together"
        );
        drop(p);
        let failed = Mutex::new(Vec::new());
        assert!(!join_or_open(
            &failed,
            4,
            cap,
            PermissionState::Denied,
            || None
        ));
        assert!(
            failed.lock().unwrap().is_empty(),
            "nothing waits on a page that never opened"
        );
    }
}
