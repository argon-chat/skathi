//! File dialogs: `<input type=file>`, the File System Access pickers and `<a download>`'s Save As.
//!
//! Each dialog runs on its own short-lived `vn-dialog` thread (an STA on Windows), so a Save As
//! left open holds up neither the shared shell thread nor the frame thread, which keeps pumping
//! while the native modal loop may message its owner synchronously.
//!
//! Owner lifecycle: a dialog belongs to the window it was asked from, one at a time (a second
//! picker is refused, a download waits its turn). When that window starts to close,
//! [`owner_closing`] marks it: a dialog not yet in the OS never will be, and the open one is
//! closed by a 100 ms check on its own thread (win.rs, mac.rs). vn-app destroys the window only
//! once [`is_idle`] says the native loop has exited, since destroying an owner under another
//! thread's modal dialog hangs or crashes. The pending promise settles as a cancel.
//!
//! `VN_DIALOGS=scripted`: nothing reaches the OS. Requests are parsed, owned and serialised as
//! usual, then wait for the devtools `dialog` command (`list`, `answer`, `cancel`); what follows
//! the answer is the real code path. Automated runs never open a window on the host.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde_json::{json, Value as Json};

use crate::fs::writable::Data;
use crate::fs::{self, fail_io, Access};
use crate::{Done, Fail};

pub mod ask;
pub mod color;
pub mod scripted;

pub use ask::{ask, ask_with, Ask, Present, Reply};

#[cfg(target_os = "windows")]
mod win;
#[cfg(target_os = "windows")]
use win as imp;

// macOS: AppKit panels on the main queue, sheets on the owner; what they are told is decided in
// mac_plan.rs, unit-tested on every OS.
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as imp;
#[cfg(target_os = "macos")]
pub use mac::ns_window_of_view;
#[cfg(any(target_os = "macos", test))]
mod mac_plan;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod imp {
    use super::*;
    pub(super) fn pick(_: &Native, _: isize, _: &Cancellation) -> Result<Option<Vec<PathBuf>>, Fail> {
        Err(Fail::new("NotSupportedError", "file dialogs are not implemented on this platform yet"))
    }
    pub(super) fn ask(_: &Ask, _: isize, _: &Cancellation) -> Result<Option<usize>, Fail> {
        Err(Fail::new("NotSupportedError", "native questions are not implemented on this platform yet"))
    }
    pub(super) fn color(_: &str, _: isize, _: &Cancellation) -> Result<Option<String>, Fail> {
        Err(Fail::new("NotSupportedError", "color pickers are not implemented on this platform yet"))
    }
    pub(super) fn known_folder(_: Known) -> Option<PathBuf> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Open,
    Save,
    Folder,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Open => "open",
            Kind::Save => "save",
            Kind::Folder => "folder",
        }
    }
}

/// One entry of the dialog's type list: a display name and its extensions (lowercase, no dot).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub exts: Vec<String>,
}

impl Filter {
    pub fn spec(&self) -> String {
        self.exts.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";")
    }
}

pub const ALL_FILES: &str = "All Files (*.*)";

/// `WellKnownDirectory` (WICG FSA), mapped to Windows known folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Known {
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

impl Known {
    pub fn parse(s: &str) -> Option<Known> {
        Some(match s {
            "desktop" => Known::Desktop,
            "documents" => Known::Documents,
            "downloads" => Known::Downloads,
            "music" => Known::Music,
            "pictures" => Known::Pictures,
            "videos" => Known::Videos,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Known::Desktop => "desktop",
            Known::Documents => "documents",
            Known::Downloads => "downloads",
            Known::Music => "music",
            Known::Pictures => "pictures",
            Known::Videos => "videos",
        }
    }
}

/// Where the dialog opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Known(Known),
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Native {
    pub kind: Kind,
    pub multiple: bool,
    pub filters: Vec<Filter>,
    pub allow_all: bool,
    pub client: u128,
    pub start: Option<Start>,
    /// `true`: `SetFolder` (startIn is an instruction). `false`: `SetDefaultFolder` (the remembered
    /// folder overrides it).
    pub force_start: bool,
    pub suggested_name: Option<String>,
    pub default_ext: Option<String>,
}

/// What happens with the user's choice.
pub enum Then {
    /// `<input type=file>`: Files and a FileList. `walk` is `webkitdirectory`; `prev` is the
    /// input's current selection, so picking the same files again is a `cancel`, not a `change`.
    Files { walk: bool, prev: Vec<PathBuf> },
    Handles,
    /// `showSaveFilePicker`: a handle, after the file is created empty or truncated.
    SaveHandle,
    Download(Data),
}

/// The extension of an `accept` token or an FSA extension: `.PNG` → `png`.
fn ext_of(token: &str) -> Option<String> {
    let e = token.strip_prefix('.')?;
    (!e.is_empty() && !e.contains(['*', '?', ';', '/', '\\', '\0'])).then(|| e.to_ascii_lowercase())
}

fn push_unique(out: &mut Vec<String>, e: String) {
    if !out.contains(&e) {
        out.push(e);
    }
}

/// `<input accept>` → the dialog's type list, as Chromium builds it: one combined entry named
/// "Image Files" / "Audio Files" / "Video Files" when the attribute is that one wildcard,
/// "Custom Files" otherwise. `tokens` are already validated by the runtime. Empty when nothing
/// maps to an extension.
pub fn filters_for_accept(tokens: &[String]) -> Vec<Filter> {
    let mut exts = Vec::new();
    let mut valid = 0;
    for t in tokens {
        let before = exts.len();
        if let Some(e) = ext_of(t) {
            push_unique(&mut exts, e);
        } else {
            for e in vn_runtime::redjs::webapi::mime::exts_for(t) {
                push_unique(&mut exts, e);
            }
        }
        if exts.len() > before {
            valid += 1;
        }
    }
    if exts.is_empty() {
        return Vec::new();
    }
    let name = match (valid, tokens.first().map(String::as_str)) {
        (1, Some("image/*")) => "Image Files",
        (1, Some("audio/*")) => "Audio Files",
        (1, Some("video/*")) => "Video Files",
        _ => "Custom Files",
    };
    vec![Filter { name: name.to_string(), exts }]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptType {
    pub description: String,
    pub accept: Vec<(String, Vec<String>)>,
}

/// FSA `types` → one dialog entry per type: its listed extensions plus those its MIME types name.
/// A type with no extension cannot filter and is left out.
pub fn filters_for_types(types: &[AcceptType]) -> Vec<Filter> {
    let mut out = Vec::new();
    for t in types {
        let mut exts = Vec::new();
        for (mime, listed) in &t.accept {
            for e in listed.iter().filter_map(|e| ext_of(e)) {
                push_unique(&mut exts, e);
            }
            for e in vn_runtime::redjs::webapi::mime::exts_for(mime) {
                push_unique(&mut exts, e);
            }
        }
        if exts.is_empty() {
            continue;
        }
        let name = if t.description.trim().is_empty() { "Custom Files".to_string() } else { t.description.clone() };
        out.push(Filter { name, exts });
    }
    out
}

/// FNV-1a, 128-bit. Must stay deterministic: the GUID names a folder remembered across sessions.
pub fn fnv128(bytes: &[u8]) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= u128::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// `SetClientGuid` for a picker: `fnv128(app id ‖ "\0" ‖ id)`, or `fnv128(app id)` with no `id`.
/// The app id is included because the dev host is one exe running many apps.
pub fn client_guid(app_id: &str, id: Option<&str>) -> u128 {
    match id {
        Some(id) => fnv128(format!("{app_id}\0{id}").as_bytes()),
        None => fnv128(app_id.as_bytes()),
    }
}

/// Downloads remember their own folder. `<` cannot appear in an FSA `id`, so no collision.
const DOWNLOAD_ID: &str = "<download>";

fn app_id() -> String {
    crate::launch::identity().map(|i| i.id.clone()).unwrap_or_else(|| "vue-native-app".to_string())
}

/// FSA's `id`: at most 32 characters of `[A-Za-z0-9_-]`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn bad(msg: impl Into<String>) -> Fail {
    Fail::type_error(msg)
}

fn strings(v: Option<&Json>) -> Vec<String> {
    v.and_then(Json::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The runtime's request → what the OS dialog is told and what follows it.
pub fn parse(req: &str, prev: Vec<PathBuf>) -> Result<(Native, Then), Fail> {
    let v: Json = serde_json::from_str(req).map_err(|e| bad(format!("dialog request is not JSON: {e}")))?;
    let kind = match v.get("kind").and_then(Json::as_str) {
        Some("open") => Kind::Open,
        Some("save") => Kind::Save,
        Some("folder") => Kind::Folder,
        other => return Err(bad(format!("dialog kind {other:?} is not open, save or folder"))),
    };
    let flag = |k: &str| v.get(k).and_then(Json::as_bool).unwrap_or(false);
    let files = v.get("result").and_then(Json::as_str) == Some("files");
    let (filters, allow_all) = match v.get("types").and_then(Json::as_array) {
        Some(types) => {
            let types: Vec<AcceptType> = types
                .iter()
                .map(|t| AcceptType {
                    description: t.get("description").and_then(Json::as_str).unwrap_or("").to_string(),
                    accept: t
                        .get("accept")
                        .and_then(Json::as_object)
                        .map(|o| o.iter().map(|(m, exts)| (m.clone(), strings(Some(exts)))).collect())
                        .unwrap_or_default(),
                })
                .collect();
            let filters = filters_for_types(&types);
            // Without a usable type, "All Files" stays so the dialog is not empty.
            let allow_all = !flag("excludeAcceptAllOption") || filters.is_empty();
            (filters, allow_all)
        }
        None => (filters_for_accept(&strings(v.get("accept"))), true),
    };
    let id = v.get("id").and_then(Json::as_str).filter(|id| valid_id(id));
    let start = match v.get("startIn") {
        Some(Json::String(s)) => Some(Start::Known(Known::parse(s).ok_or_else(|| bad(format!("{s:?} is not a WellKnownDirectory")))?)),
        Some(Json::Object(o)) => {
            let h = o.get("h").and_then(Json::as_u64).unwrap_or(0);
            let (path, k) = fs::path_of(h).ok_or_else(|| Fail::new("InvalidStateError", fs::INVALID_STATE))?;
            // A file handle starts the dialog in the file's folder (spec).
            let dir = match k {
                fs::Kind::Directory => path,
                fs::Kind::File => path.parent().map(Path::to_path_buf).unwrap_or(path),
            };
            Some(Start::Path(dir))
        }
        _ => None,
    };
    let suggested_name = v.get("suggestedName").and_then(Json::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let default_ext = match kind {
        Kind::Save => filters.first().and_then(|f| f.exts.first()).cloned(),
        _ => None,
    };
    let native = Native {
        kind,
        multiple: kind == Kind::Open && flag("multiple"),
        filters,
        allow_all,
        client: client_guid(&app_id(), id),
        force_start: start.is_some(),
        start,
        suggested_name,
        default_ext,
    };
    let then = match (kind, files) {
        (Kind::Save, _) => Then::SaveHandle,
        (_, true) => Then::Files { walk: kind == Kind::Folder, prev },
        (_, false) => Then::Handles,
    };
    Ok((native, then))
}

const RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1",
    "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A download's file name, as Chromium's `net::GenerateFileName` makes it: the `download`
/// attribute if anything is left after sanitising, else the URL's own; characters Windows refuses
/// become `_`, a device name gets a `_` prefix, and a name with no extension gets its MIME type's.
pub fn download_name(attr: &str, url: &str, mime: &str) -> String {
    fn clean(s: &str) -> String {
        let s: String = s
            .chars()
            .map(|c| if (c as u32) < 0x20 || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{7f}') { '_' } else { c })
            .collect();
        s.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string()
    }
    // A path in the attribute keeps its last component only, as in Chromium.
    let leaf = attr.rsplit(['/', '\\']).next().unwrap_or(attr);
    let mut name = clean(leaf);
    if name.is_empty() {
        name = if url.starts_with("data:") {
            "download".to_string()
        } else {
            clean(url.rsplit('/').next().unwrap_or(""))
        };
    }
    if name.is_empty() {
        name = "download".to_string();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    if RESERVED_NAMES.contains(&stem.trim()) {
        name = format!("_{name}");
    }
    if !name.contains('.') {
        if let Some(ext) = vn_runtime::redjs::webapi::mime::exts_for(mime).first() {
            name = format!("{name}.{ext}");
        }
    }
    name
}

/// A resolved `blob:` URL as the bytes a download writes. A memory Blob shares its buffer; a
/// picked file is copied on the dialog thread, and one changed since the pick fails with
/// `NotReadableError`.
pub fn data_of_blob_target(t: vn_runtime::redjs::webapi::url::BlobTarget) -> Data {
    use vn_runtime::redjs::webapi::blob::{DiskFile, Range, Source};
    use vn_runtime::redjs::webapi::url::BlobTarget;
    match t {
        BlobTarget::Bytes { bytes, offset, len, .. } => {
            Data::Blob(Range { source: Source::Memory(bytes), offset: offset as u64, len: len as u64 })
        }
        BlobTarget::Path { path, range, size, mtime, .. } => {
            let (offset, len) = range.unwrap_or((0, size));
            Data::Blob(Range { source: Source::Disk(Arc::new(DiskFile { path, size, mtime })), offset, len })
        }
    }
}

/// The window a dialog belongs to: vn-tree's window key and its native window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    pub key: u64,
    pub hwnd: isize,
}

static RESOLVER: OnceLock<fn() -> Option<Owner>> = OnceLock::new();

/// vn-app names the published window at the moment of a call. Without a resolver (a unit test, a
/// headless tool) a dialog has no owner.
pub fn set_owner_resolver(f: fn() -> Option<Owner>) {
    let _ = RESOLVER.set(f);
}

#[derive(Default)]
struct OwnerState {
    closed: AtomicBool,
    /// A dialog of this owner is in the OS (or waiting for its scripted answer).
    active: AtomicBool,
    /// Downloads wait in click order: the next ticket to hand out, and the tickets waiting.
    ticket: std::sync::atomic::AtomicU64,
    queue: Mutex<std::collections::VecDeque<u64>>,
}

impl OwnerState {
    fn leave_queue(&self, ticket: u64) {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).retain(|&t| t != ticket);
    }
}

fn owners() -> &'static Mutex<std::collections::HashMap<u64, Arc<OwnerState>>> {
    static O: OnceLock<Mutex<std::collections::HashMap<u64, Arc<OwnerState>>>> = OnceLock::new();
    O.get_or_init(Default::default)
}

fn owner_state(key: u64) -> Arc<OwnerState> {
    owners().lock().unwrap_or_else(|p| p.into_inner()).entry(key).or_default().clone()
}

/// Window `key` is about to be destroyed. Its open dialog is told to close. Returns `true` while
/// a dialog is still running on it; destroy the window only once [`is_idle`].
pub fn owner_closing(key: u64) -> bool {
    let mut map = owners().lock().unwrap_or_else(|p| p.into_inner());
    let Some(st) = map.get(&key).cloned() else { return false };
    st.closed.store(true, Ordering::Release);
    let active = st.active.load(Ordering::Acquire);
    if !active {
        map.remove(&key);
    }
    active
}

/// Has window `key`'s dialog left the OS? A closed owner's entry is dropped once idle.
pub fn is_idle(key: u64) -> bool {
    let mut map = owners().lock().unwrap_or_else(|p| p.into_inner());
    let Some(st) = map.get(&key) else { return true };
    let idle = !st.active.load(Ordering::Acquire);
    if idle && st.closed.load(Ordering::Acquire) {
        map.remove(&key);
    }
    idle
}

#[derive(Clone)]
pub struct Cancellation(Arc<OwnerState>);

impl Cancellation {
    pub fn is_requested(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
    }
}

/// `VN_DIALOGS=scripted`. Unit tests are always scripted, so none opens a window on the host.
pub fn scripted_mode() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    cfg!(test) || *S.get_or_init(|| std::env::var("VN_DIALOGS").is_ok_and(|v| v.eq_ignore_ascii_case("scripted")))
}

/// Chromium's message; the runtime prefixes "Failed to execute …".
pub const ALREADY_ACTIVE: &str = "File picker already active.";

/// `__vn_dialog_open(json, currentFiles)` and `__vn_download(…)`: one request. A picker is
/// refused at once if its owner already has a dialog open; a download waits for it, as Chromium
/// shows successive Save As dialogs.
pub fn start(native: Native, then: Then, label: &'static str) -> String {
    start_owned(RESOLVER.get().and_then(|f| f()), native, then, label)
}

const QUEUE_POLL: std::time::Duration = std::time::Duration::from_millis(25);

/// On a dialog thread: waits for `ticket`'s turn, once the owner has no dialog open, then claims
/// the owner. `false` when the owner closed meanwhile: nothing is claimed, so the HWND is never
/// used.
fn wait_turn(st: &OwnerState, ticket: u64, cancel: &Cancellation) -> bool {
    loop {
        if cancel.is_requested() {
            st.leave_queue(ticket);
            return false;
        }
        let front = st.queue.lock().unwrap_or_else(|p| p.into_inner()).front() == Some(&ticket);
        if front && !st.active.swap(true, Ordering::AcqRel) {
            break;
        }
        std::thread::sleep(QUEUE_POLL);
    }
    st.leave_queue(ticket);
    if cancel.is_requested() {
        st.active.store(false, Ordering::Release);
        return false;
    }
    true
}

fn start_owned(owner: Option<Owner>, native: Native, then: Then, label: &'static str) -> String {
    let key = owner.map(|o| o.key).unwrap_or(0);
    let hwnd = owner.map(|o| o.hwnd).unwrap_or(0);
    let st = owner_state(key);
    let token = crate::next_token();
    if st.closed.load(Ordering::Acquire) {
        crate::settle(token, Ok(cancelled()));
        return format!("{{\"token\":{token}}}");
    }
    let queued = matches!(then, Then::Download(_));
    if !queued && st.active.swap(true, Ordering::AcqRel) {
        return Fail::new("NotAllowedError", ALREADY_ACTIVE).code("EBUSY").to_sync_json();
    }
    let ticket = st.ticket.fetch_add(1, Ordering::AcqRel);
    if queued {
        st.queue.lock().unwrap_or_else(|p| p.into_inner()).push_back(ticket);
    }
    let cancel = Cancellation(st.clone());
    let spawned = std::thread::Builder::new().name("vn-dialog".into()).spawn(move || {
        if queued && !wait_turn(&st, ticket, &cancel) {
            crate::settle(token, Ok(cancelled()));
            return;
        }
        let picked = if scripted_mode() {
            scripted::pick(&native, key, label, &cancel)
        } else {
            imp::pick(&native, hwnd, &cancel)
        };
        // The native loop has exited: the owner may be destroyed, and a new dialog may start.
        st.active.store(false, Ordering::Release);
        let result = match picked {
            Ok(Some(paths)) if !paths.is_empty() && !cancel.is_requested() => finish(paths, then),
            Ok(_) => Ok(Done::json(&cancelled())),
            Err(f) => Err(f),
        };
        crate::settle_done(token, result);
    });
    if spawned.is_err() {
        // The closure went down with the failed spawn; a queued download claimed only its queue slot.
        let st = owner_state(key);
        if queued {
            st.leave_queue(ticket);
        } else {
            st.active.store(false, Ordering::Release);
        }
        return Fail::new("InvalidStateError", "could not start the dialog thread").code("EIO").to_sync_json();
    }
    format!("{{\"token\":{token}}}")
}

fn cancelled() -> String {
    "{\"cancelled\":true}".to_string()
}

pub fn start_open(req: &str, prev: Vec<PathBuf>) -> String {
    if let Ok(request) = serde_json::from_str::<Json>(req) {
        if request["kind"] == "color" { return color::start(request["value"].as_str().unwrap_or("#000000")); }
    }
    match parse(req, prev) {
        Ok((native, then)) => start(native, then, "picker"),
        Err(f) => f.to_sync_json(),
    }
}

/// `__vn_download`: Save As for bytes the runtime already resolved, starting in Downloads.
pub fn start_download(data: Data, name: String) -> String {
    let native = Native {
        kind: Kind::Save,
        multiple: false,
        filters: Vec::new(),
        allow_all: true,
        client: client_guid(&app_id(), Some(DOWNLOAD_ID)),
        start: Some(Start::Known(Known::Downloads)),
        force_start: false,
        default_ext: name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).filter(|e| !e.is_empty()),
        suggested_name: Some(name),
    };
    start(native, Then::Download(data), "download")
}

struct Picked {
    path: PathBuf,
    name: String,
    size: u64,
    mtime: Option<SystemTime>,
    rel: String,
}

fn stat_picked(path: PathBuf, rel: String) -> Result<Picked, Fail> {
    let (size, mtime) = fs::op_stat_file(&path)?;
    let name = fs::name_of(&path);
    Ok(Picked { path, name, size, mtime, rel })
}

/// `webkitdirectory`: every file under `root`, sorted by relative path. Directory symlinks and
/// junctions are not followed (a link up the tree would never end). `webkitRelativePath` is
/// `<picked folder>/<path/inside>` with `/`.
pub fn walk(root: &Path) -> Result<Vec<(PathBuf, String)>, Fail> {
    let base = fs::name_of(root);
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), base)];
    while let Some((dir, rel)) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| fail_io(&e, Access::Read))?.filter_map(Result::ok);
        let mut subdirs = Vec::new();
        for e in entries {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let child_rel = format!("{rel}/{name}");
            if ft.is_dir() {
                subdirs.push((e.path(), child_rel));
            } else if ft.is_file() {
                out.push((e.path(), child_rel));
            } else if ft.is_symlink() && std::fs::metadata(e.path()).is_ok_and(|m| m.is_file()) {
                out.push((e.path(), child_rel));
            }
        }
        stack.extend(subdirs);
    }
    // Sorted, so what the app lists is stable from run to run.
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

fn finish(paths: Vec<PathBuf>, then: Then) -> Result<Done, Fail> {
    match then {
        Then::Files { walk: walked, prev } => {
            let picked: Vec<Picked> = if walked {
                let root = paths.into_iter().next().ok_or_else(|| Fail::new("AbortError", "nothing was picked"))?;
                walk(&root)?.into_iter().map(|(p, rel)| stat_picked(p, rel)).collect::<Result<_, _>>()?
            } else {
                paths.into_iter().map(|p| stat_picked(p, String::new())).collect::<Result<_, _>>()?
            };
            let same = picked.len() == prev.len() && picked.iter().zip(&prev).all(|(a, b)| &a.path == b);
            if same {
                return Ok(Done::json(&cancelled()));
            }
            let payload = json!({ "files": picked.len() }).to_string();
            Ok(Done::with_value(payload, move |g| unsafe { make_file_list(g, &picked) }))
        }
        Then::Handles => {
            let mut handles = Vec::new();
            for p in paths {
                let meta = std::fs::metadata(&p).map_err(|e| fail_io(&e, Access::Read))?;
                let kind = if meta.is_dir() { fs::Kind::Directory } else { fs::Kind::File };
                handles.push(fs::granted_handle_json(p, kind));
            }
            Ok(Done { payload: json!({ "handles": handles }).to_string(), make: None })
        }
        Then::SaveHandle => {
            let p = paths.into_iter().next().ok_or_else(|| Fail::new("AbortError", "nothing was picked"))?;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&p)
                .map_err(|e| fail_io(&e, Access::Write))?;
            Ok(Done { payload: json!({ "handles": [fs::granted_handle_json(p, fs::Kind::File)] }).to_string(), make: None })
        }
        Then::Download(data) => {
            let p = paths.into_iter().next().ok_or_else(|| Fail::new("AbortError", "nothing was picked"))?;
            crate::fs::writable::write_whole(&p, &data)?;
            let name = fs::name_of(&p);
            crate::log_later("dialog", format!("[download] saved {name}"));
            Ok(Done::json(&json!({ "saved": name }).to_string()))
        }
    }
}

/// The picked Files and their FileList, on the JS thread; each File is protected until the list
/// holds it (a Rust `Vec` is not a GC root).
unsafe fn make_file_list(g: *mut std::ffi::c_void, picked: &[Picked]) -> u64 {
    use vn_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
    use vn_runtime::redjs::webapi::file;
    let mut files = Vec::with_capacity(picked.len());
    for p in picked {
        let f = file::create_file(g, &p.path, &p.name, None, p.size, p.mtime, &p.rel);
        redjs_protect(g, f);
        files.push(f);
    }
    let list = file::create_file_list(g, &files);
    for f in files {
        redjs_unprotect(g, f);
    }
    list
}

/// A request, described for the devtools (`dialog list`) and the log.
pub fn describe(n: &Native) -> Json {
    let start = match &n.start {
        Some(Start::Known(k)) => Json::String(k.as_str().to_string()),
        Some(Start::Path(p)) => Json::String(p.to_string_lossy().into_owned()),
        None => Json::Null,
    };
    json!({
        "kind": n.kind.as_str(),
        "multiple": n.multiple,
        "filters": n.filters.iter().map(|f| json!({ "name": f.name, "spec": f.spec() })).collect::<Vec<_>>(),
        "allowAll": n.allow_all,
        "startIn": start,
        "suggestedName": n.suggested_name,
        "defaultExtension": n.default_ext,
        "clientGuid": format!("{:032x}", n.client),
    })
}

pub fn known_folder(k: Known) -> Option<PathBuf> {
    imp::known_folder(k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::tests::Scratch;
    use crate::tests::{token_of, wait_for_settles};
    use std::sync::atomic::AtomicU64;

    fn toks(s: &[&str]) -> Vec<String> {
        s.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn accept_becomes_one_combined_filter_named_as_chromium_names_it() {
        let image = filters_for_accept(&toks(&["image/*"]));
        assert_eq!(image.len(), 1);
        assert_eq!(image[0].name, "Image Files");
        assert!(image[0].exts.contains(&"png".to_string()) && image[0].exts.contains(&"jpg".to_string()));
        assert_eq!(filters_for_accept(&toks(&["audio/*"]))[0].name, "Audio Files");
        assert_eq!(filters_for_accept(&toks(&["video/*"]))[0].name, "Video Files");
        let one = filters_for_accept(&toks(&[".csv"]));
        assert_eq!(one, vec![Filter { name: "Custom Files".into(), exts: vec!["csv".into()] }]);
        let mixed = filters_for_accept(&toks(&[".png", "image/*", "application/pdf"]));
        assert_eq!(mixed[0].name, "Custom Files");
        assert_eq!(mixed[0].exts[0], "png");
        assert_eq!(mixed[0].exts.iter().filter(|e| *e == "png").count(), 1);
        assert!(mixed[0].exts.contains(&"pdf".to_string()));
        assert!(mixed[0].spec().starts_with("*.png;"));
        assert!(filters_for_accept(&toks(&["x-unknown/nothing"])).is_empty());
        assert!(filters_for_accept(&[]).is_empty());
    }

    #[test]
    fn fsa_types_become_one_filter_each_with_their_mime_extensions() {
        let types = vec![
            AcceptType { description: "Text".into(), accept: vec![("text/plain".into(), vec![".txt".into(), ".LOG".into()])] },
            AcceptType { description: "".into(), accept: vec![("image/png".into(), vec![])] },
            AcceptType { description: "Nothing".into(), accept: vec![("x-unknown/nothing".into(), vec![])] },
        ];
        let f = filters_for_types(&types);
        assert_eq!(f.len(), 2, "a type with no extension is left out: {f:?}");
        assert_eq!(f[0].name, "Text");
        assert_eq!(&f[0].exts[..2], &["txt".to_string(), "log".to_string()]);
        assert_eq!(f[1].name, "Custom Files");
        assert!(f[1].exts.contains(&"png".to_string()));
    }

    #[test]
    fn the_client_guid_is_fnv1a_128_and_scoped_to_the_app() {
        assert_eq!(fnv128(b""), 0x6c62272e07bb014262b821756295c58d);
        assert_eq!(fnv128(b"a"), 0xd228cb696f1a8caf78912b704e4a8964);
        assert_eq!(client_guid("app", Some("x")), fnv128(b"app\0x"));
        assert_eq!(client_guid("app", None), fnv128(b"app"));
        assert_ne!(client_guid("app", Some("x")), client_guid("other", Some("x")));
        assert_ne!(client_guid("app", Some("x")), client_guid("app", Some("y")));
        assert!(valid_id("Recent_files-2"));
        for bad in ["", "has space", "a/b", &"x".repeat(33)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn well_known_directories_map_to_known_folders() {
        for (s, k) in [
            ("desktop", Known::Desktop),
            ("documents", Known::Documents),
            ("downloads", Known::Downloads),
            ("music", Known::Music),
            ("pictures", Known::Pictures),
            ("videos", Known::Videos),
        ] {
            assert_eq!(Known::parse(s), Some(k));
            assert_eq!(k.as_str(), s);
        }
        assert_eq!(Known::parse("Documents"), None, "the enum is case-sensitive, as in WebIDL");
        if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
            assert!(known_folder(Known::Downloads).is_some_and(|p| p.is_absolute()));
        }
    }

    #[test]
    fn a_request_parses_into_the_dialog_and_what_follows_it() {
        let (n, then) = parse(r#"{"kind":"open","multiple":true,"accept":[".txt"],"result":"files"}"#, vec![]).unwrap();
        assert_eq!((n.kind, n.multiple, n.allow_all), (Kind::Open, true, true));
        assert!(matches!(then, Then::Files { walk: false, .. }));
        let (n, then) = parse(r#"{"kind":"folder","multiple":true,"result":"files"}"#, vec![]).unwrap();
        assert!(!n.multiple, "only an Open picks several");
        assert!(matches!(then, Then::Files { walk: true, .. }));
        let (n, then) = parse(
            r#"{"kind":"save","types":[{"description":"CSV","accept":{"text/csv":[".csv"]}}],
                "excludeAcceptAllOption":true,"suggestedName":"out.csv","id":"export","startIn":"documents"}"#,
            vec![],
        )
        .unwrap();
        assert!(matches!(then, Then::SaveHandle));
        assert_eq!(n.default_ext.as_deref(), Some("csv"));
        assert!(!n.allow_all);
        assert_eq!(n.start, Some(Start::Known(Known::Documents)));
        assert!(n.force_start);
        assert_eq!(n.client, client_guid(&app_id(), Some("export")));
        let s = Scratch::new("dialog-start");
        std::fs::write(s.0.join("f.txt"), b"x").unwrap();
        let h = fs::register(s.0.join("f.txt"), fs::Kind::File);
        let (n, _) = parse(&format!(r#"{{"kind":"open","startIn":{{"h":{h}}}}}"#), vec![]).unwrap();
        assert_eq!(n.start, Some(Start::Path(s.0.clone())));
        fs::release(h);
        assert_eq!(parse(&format!(r#"{{"kind":"open","startIn":{{"h":{h}}}}}"#), vec![]).err().map(|f| f.name), Some("InvalidStateError"));
        assert_eq!(parse(r#"{"kind":"open","startIn":"home"}"#, vec![]).err().map(|f| f.name), Some("TypeError"));
        assert_eq!(parse(r#"{"kind":"run"}"#, vec![]).err().map(|f| f.name), Some("TypeError"));
        let (n, _) = parse(r#"{"kind":"open","types":[],"excludeAcceptAllOption":true}"#, vec![]).unwrap();
        assert!(n.allow_all);
    }

    #[test]
    fn download_names_follow_chromium() {
        assert_eq!(download_name("report.csv", "blob:x/1", "text/csv"), "report.csv");
        assert_eq!(download_name("dir/sub\\a:b.txt", "blob:x/1", ""), "a_b.txt");
        assert_eq!(download_name("", "data:text/plain,hi", "text/plain"), "download.txt");
        assert_eq!(
            download_name("", "blob:http://localhost/5b1f-77aa", "application/pdf"),
            "5b1f-77aa.pdf"
        );
        assert_eq!(download_name(" ..x.. ", "blob:x/1", ""), "x");
        assert_eq!(download_name("con.txt", "blob:x/1", ""), "_con.txt");
        assert_eq!(download_name("a?b", "data:,x", "application/x-unknown"), "a_b");
    }

    static KEY: AtomicU64 = AtomicU64::new(1 << 40);

    fn owner() -> Option<Owner> {
        Some(Owner { key: KEY.fetch_add(1, Ordering::Relaxed), hwnd: 0 })
    }

    fn pending_of(window: u64) -> u64 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Some(v) = scripted::list().as_array().unwrap().iter().find(|v| v["window"] == window) {
                return v["id"].as_u64().unwrap();
            }
            assert!(std::time::Instant::now() < deadline, "no scripted dialog appeared for window {window}");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn open(owner: Option<Owner>, req: &str, prev: Vec<PathBuf>) -> String {
        let (n, then) = parse(req, prev).unwrap();
        start_owned(owner, n, then, "picker")
    }

    #[test]
    fn a_scripted_open_answers_cancels_and_refuses_a_second_dialog() {
        let s = Scratch::new("dialog-open");
        let (a, b) = (s.0.join("a.txt"), s.0.join("b.txt"));
        std::fs::write(&a, b"alpha").unwrap();
        std::fs::write(&b, b"beta").unwrap();
        let o = owner();
        let key = o.unwrap().key;
        let req = r#"{"kind":"open","multiple":true,"result":"files"}"#;
        let t = token_of(&open(o, req, vec![]));
        let id = pending_of(key);
        let second: Json = serde_json::from_str(&open(o, req, vec![])).unwrap();
        assert_eq!((second["name"].as_str(), second["error"].as_str()), (Some("NotAllowedError"), Some(ALREADY_ACTIVE)));
        // Answers the OS dialog would not have taken are refused, and the dialog keeps waiting.
        assert!(scripted::answer(id, vec![s.0.join("missing.txt")]).is_err());
        assert!(scripted::answer(id, vec![PathBuf::from("relative.txt")]).is_err());
        scripted::answer(id, vec![a.clone(), b.clone()]).unwrap();
        let got = wait_for_settles(&[t]);
        assert_eq!(got[&t], (true, r#"{"files":2}"#.to_string()));
        assert!(is_idle(key));
        let t = token_of(&open(o, req, vec![a.clone(), b.clone()]));
        scripted::answer(pending_of(key), vec![a.clone(), b.clone()]).unwrap();
        assert_eq!(wait_for_settles(&[t])[&t], (true, cancelled()));
        let t = token_of(&open(o, req, vec![]));
        scripted::cancel(pending_of(key)).unwrap();
        assert_eq!(wait_for_settles(&[t])[&t], (true, cancelled()));
        let t = token_of(&open(o, r#"{"kind":"open","result":"handles"}"#, vec![]));
        let id = pending_of(key);
        assert!(scripted::answer(id, vec![a.clone(), b.clone()]).is_err());
        scripted::answer(id, vec![a.clone()]).unwrap();
        let (ok, payload) = wait_for_settles(&[t]).remove(&t).unwrap();
        assert!(ok);
        let v: Json = serde_json::from_str(&payload).unwrap();
        assert_eq!((v["handles"][0]["name"].as_str(), v["handles"][0]["kind"].as_str()), (Some("a.txt"), Some("file")));
        assert!(!payload.contains(&*s.0.to_string_lossy()), "no path in what JS sees: {payload}");
        fs::release(v["handles"][0]["h"].as_u64().unwrap());
    }

    #[test]
    fn a_closing_owner_cancels_its_dialog_and_refuses_new_ones() {
        let o = owner();
        let key = o.unwrap().key;
        let t = token_of(&open(o, r#"{"kind":"open","result":"handles"}"#, vec![]));
        let _ = pending_of(key);
        assert!(!is_idle(key));
        assert!(owner_closing(key), "a dialog is still running on it");
        assert_eq!(wait_for_settles(&[t])[&t], (true, cancelled()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !is_idle(key) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!owner_closing(KEY.fetch_add(1, Ordering::Relaxed)));
        // A request from a closed owner (between its close and its destruction) is a cancel.
        let o2 = owner();
        let k2 = o2.unwrap().key;
        owner_state(k2).closed.store(true, Ordering::Release);
        let t = token_of(&open(o2, r#"{"kind":"open","result":"handles"}"#, vec![]));
        assert_eq!(wait_for_settles(&[t])[&t], (true, cancelled()));
    }

    #[test]
    fn a_folder_pick_walks_without_following_links_and_names_relative_paths() {
        let s = Scratch::new("dialog-walk");
        let root = s.0.join("proj");
        std::fs::create_dir_all(root.join("a/y")).unwrap();
        std::fs::write(root.join("c.txt"), b"c").unwrap();
        std::fs::write(root.join("b.txt"), b"b").unwrap();
        std::fs::write(root.join("a/x.txt"), b"x").unwrap();
        std::fs::write(root.join("a/y/z.txt"), b"z").unwrap();
        let rel: Vec<String> = walk(&root).unwrap().into_iter().map(|(_, r)| r).collect();
        assert_eq!(rel, ["proj/a/x.txt", "proj/a/y/z.txt", "proj/b.txt", "proj/c.txt"]);
        let o = owner();
        let t = token_of(&open(o, r#"{"kind":"folder","result":"files"}"#, vec![]));
        let id = pending_of(o.unwrap().key);
        assert!(scripted::answer(id, vec![root.join("b.txt")]).is_err(), "a folder pick takes a folder");
        scripted::answer(id, vec![root.clone()]).unwrap();
        assert_eq!(wait_for_settles(&[t])[&t], (true, r#"{"files":4}"#.to_string()));
    }

    #[test]
    fn a_save_pick_truncates_and_a_download_writes_through_a_swap() {
        let s = Scratch::new("dialog-save");
        let target = s.0.join("out.txt");
        std::fs::write(&target, b"old contents").unwrap();
        let o = owner();
        let key = o.unwrap().key;
        let t = token_of(&open(o, r#"{"kind":"save","suggestedName":"out.txt"}"#, vec![]));
        let id = pending_of(key);
        assert!(scripted::answer(id, vec![s.0.join("no-such-dir").join("x.txt")]).is_err());
        scripted::answer(id, vec![target.clone()]).unwrap();
        let (ok, payload) = wait_for_settles(&[t]).remove(&t).unwrap();
        assert!(ok, "{payload}");
        assert_eq!(std::fs::read(&target).unwrap(), b"", "FSA step 7.7: the picked file starts empty");
        let v: Json = serde_json::from_str(&payload).unwrap();
        fs::release(v["handles"][0]["h"].as_u64().unwrap());

        let saved = s.0.join("report.csv");
        let n = Native {
            kind: Kind::Save,
            multiple: false,
            filters: vec![],
            allow_all: true,
            client: 0,
            start: Some(Start::Known(Known::Downloads)),
            force_start: false,
            suggested_name: Some("report.csv".into()),
            default_ext: Some("csv".into()),
        };
        let t = token_of(&start_owned(o, n, Then::Download(Data::Bytes(b"a,b\n1,2\n".to_vec())), "download"));
        let id = pending_of(key);
        let listed = scripted::list();
        let v = listed.as_array().unwrap().iter().find(|v| v["id"] == id).unwrap();
        assert_eq!((v["what"].as_str(), v["startIn"].as_str()), (Some("download"), Some("downloads")));
        assert_eq!(v["suggestedName"], "report.csv");
        scripted::answer(id, vec![saved.clone()]).unwrap();
        assert_eq!(wait_for_settles(&[t])[&t], (true, r#"{"saved":"report.csv"}"#.to_string()));
        assert_eq!(std::fs::read(&saved).unwrap(), b"a,b\n1,2\n");
        let names: Vec<String> =
            std::fs::read_dir(&s.0).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!names.iter().any(|n| n.contains(".vnswap")), "no swap left: {names:?}");
    }

    fn download(o: Option<Owner>, name: &str, bytes: &[u8]) -> String {
        let n = Native {
            kind: Kind::Save,
            multiple: false,
            filters: vec![],
            allow_all: true,
            client: 0,
            start: None,
            force_start: false,
            suggested_name: Some(name.into()),
            default_ext: None,
        };
        start_owned(o, n, Then::Download(Data::Bytes(bytes.to_vec())), "download")
    }

    fn pending_count(window: u64) -> usize {
        scripted::list().as_array().unwrap().iter().filter(|v| v["window"] == window).count()
    }

    #[test]
    fn a_download_waits_for_the_open_dialog_instead_of_being_refused() {
        let s = Scratch::new("dialog-queue");
        let o = owner();
        let key = o.unwrap().key;
        let picker = token_of(&open(o, r#"{"kind":"open","result":"handles"}"#, vec![]));
        let first = pending_of(key);
        let d1 = token_of(&download(o, "one.txt", b"1"));
        let d2 = token_of(&download(o, "two.txt", b"2"));
        std::thread::sleep(QUEUE_POLL * 3);
        assert_eq!(pending_count(key), 1, "only the picker is in front of the user");
        scripted::cancel(first).unwrap();
        assert_eq!(wait_for_settles(&[picker])[&picker], (true, cancelled()));
        for name in ["one.txt", "two.txt"] {
            let id = pending_of(key);
            assert_eq!(pending_count(key), 1);
            let listed = scripted::list();
            let v = listed.as_array().unwrap().iter().find(|v| v["id"] == id).unwrap();
            assert_eq!(v["suggestedName"], name);
            scripted::answer(id, vec![s.0.join(name)]).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while scripted::list().as_array().unwrap().iter().any(|v| v["id"] == id) || !s.0.join(name).exists() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let got = wait_for_settles(&[d1, d2]);
        assert_eq!(got[&d1], (true, r#"{"saved":"one.txt"}"#.to_string()));
        assert_eq!(got[&d2], (true, r#"{"saved":"two.txt"}"#.to_string()));
        assert_eq!(std::fs::read(s.0.join("two.txt")).unwrap(), b"2");
        // A queued download on a window that closes is a cancel, and never shown.
        let o = owner();
        let key = o.unwrap().key;
        let picker = token_of(&open(o, r#"{"kind":"open","result":"handles"}"#, vec![]));
        let _ = pending_of(key);
        let queued = token_of(&download(o, "never.txt", b"x"));
        assert!(owner_closing(key));
        let got = wait_for_settles(&[picker, queued]);
        assert_eq!((got[&picker].1.as_str(), got[&queued].1.as_str()), (cancelled().as_str(), cancelled().as_str()));
        assert!(!s.0.join("never.txt").exists());
    }
}
