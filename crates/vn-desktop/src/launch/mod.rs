//! How this process was launched, and the one-instance rule (docs/LAUNCH.md).
//!
//! ```text
//!   main()  ──args::process()──▶ start(cfg) ──┬─ not single-instance ─▶ run as the app
//!                                             ├─ first instance ──────▶ run, + pipe server
//!                                             └─ another one runs ────▶ forward argv/url/cwd,
//!                                                                      exit 0 (2: no answer,
//!                                                                      3: pipe not ours)
//!   pipe server (vn-instance) ── verify client ── read ≤ 1 MiB ── ack ── raise ── emit(launch)
//!                                                                                │
//!   JS: appLaunch "launch" event + launchQueue LaunchParams ◀── pump(), token 0 ─┘
//! ```
//!
//! `start` is the first thing `main()` does after the argv parse — before the
//! inspector, the logger, the payload's assets or gpui — so a second launch
//! never creates a window, never touches `localStorage`, and is gone in
//! milliseconds.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use serde_json::{json, Value as Json};

use crate::args::Args;

#[cfg(target_os = "windows")]
mod instance_win;
#[cfg(target_os = "windows")]
use instance_win as instance;

pub mod protocol;
#[cfg(target_os = "windows")]
pub mod protocol_win;

// ---------------------------------------------------------------------------
// configuration: the `launch` block of the manifest / payload META
// ---------------------------------------------------------------------------

/// `vn.config.json`'s launch keys, as the builder copied them (`build.ts` into
/// `dist/manifest.json`, `vn build` into the payload's META).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `"id"` — reverse-DNS, `gl.argon.notes`. `None`: the app name stands in.
    pub id: Option<String>,
    /// `"singleInstance"` — default false.
    pub single_instance: bool,
    /// `"raiseOnSecondInstance"` — default true.
    pub raise: bool,
    /// `"protocols"` — the custom schemes this app handles. Default none.
    pub protocols: Vec<String>,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        LaunchConfig { id: None, single_instance: false, raise: true, protocols: Vec::new() }
    }
}

impl LaunchConfig {
    /// Parses the launch block. A key of the wrong type is IGNORED with a line
    /// on stderr — the builders validate, so reaching this means a hand-edited
    /// manifest, and the app should still start.
    pub fn parse(text: Option<&str>) -> LaunchConfig {
        let mut cfg = LaunchConfig::default();
        let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else { return cfg };
        let v: Json = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[launch] the launch block is not JSON ({e}); using the defaults");
                return cfg;
            }
        };
        let bad = |k: &str, want: &str| eprintln!("[launch] `{k}` must be {want}; ignored");
        match v.get("id") {
            None | Some(Json::Null) => {}
            Some(Json::String(s)) if !s.trim().is_empty() => cfg.id = Some(s.trim().to_string()),
            Some(_) => bad("id", "a non-empty string"),
        }
        match v.get("singleInstance") {
            None | Some(Json::Null) => {}
            Some(Json::Bool(b)) => cfg.single_instance = *b,
            Some(_) => bad("singleInstance", "true or false"),
        }
        match v.get("raiseOnSecondInstance") {
            None | Some(Json::Null) => {}
            Some(Json::Bool(b)) => cfg.raise = *b,
            Some(_) => bad("raiseOnSecondInstance", "true or false"),
        }
        match v.get("protocols") {
            None | Some(Json::Null) => {}
            Some(Json::Array(list)) => {
                for p in list {
                    match p.as_str() {
                        Some(s) if !s.trim().is_empty() => cfg.protocols.push(s.trim().to_ascii_lowercase()),
                        _ => bad("protocols[…]", "a scheme string"),
                    }
                }
            }
            Some(_) => bad("protocols", "an array of scheme strings"),
        }
        cfg
    }
}

// ---------------------------------------------------------------------------
// identity
// ---------------------------------------------------------------------------

/// The app's name and id. The NAME chain is vn-storage's (`VN_APP_NAME` → the
/// build's `name` → the exe stem) so the two can never disagree about which
/// app this is; the id is `vn.config.json`'s `"id"`, defaulting to the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub id: String,
}

pub fn resolve_identity(cfg: &LaunchConfig, build_name: Option<String>) -> Identity {
    let name = std::env::var("VN_APP_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or(build_name.filter(|s| !s.trim().is_empty()))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "vue-native-app".to_string());
    let id = cfg.id.clone().unwrap_or_else(|| name.clone());
    Identity { name, id }
}

// ---------------------------------------------------------------------------
// what this launch was
// ---------------------------------------------------------------------------

/// The launch, as `appLaunch` and `launchQueue` see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchInfo {
    /// Electron's `process.argv` shape (exe first), minus the `--vn-*` internals.
    pub argv: Vec<String>,
    pub cwd: String,
    /// The deep link, when this is a protocol launch.
    pub url: Option<String>,
    /// The app arguments that name an existing file or folder, absolute.
    pub paths: Vec<String>,
}

impl LaunchInfo {
    pub fn to_json(&self) -> Json {
        json!({ "argv": self.argv, "cwd": self.cwd, "url": self.url, "paths": self.paths })
    }
}

/// `args` that resolve to an existing file or folder against `cwd`, made
/// absolute. Flags (`-x`, `--y`) are never paths, and neither is a URL.
pub fn existing_paths(args: &[String], cwd: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        if a.is_empty() || a.starts_with('-') || crate::shell::scheme_of(a).is_some_and(|s| s.len() > 1) {
            continue;
        }
        let p = Path::new(a);
        let joined = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
        // Lexically normalised (`.` and `..` folded, no `\\?\` prefix) — what
        // Explorer would show, and what `canonicalize` would not.
        let abs = std::path::absolute(&joined).unwrap_or(joined);
        if std::fs::symlink_metadata(&abs).is_ok() {
            out.push(abs.to_string_lossy().into_owned());
        }
    }
    out
}

/// `LaunchParams.files`: one File System Access handle per launch path,
/// `{h, name, kind}`, granted here because the user named these paths on the
/// command line (or in Explorer's "Open with"), which is the launch-time
/// equivalent of picking them. A path's kind is its target's: a symlink to a
/// folder is a directory handle. A path that vanished between the parse and
/// now (or a dangling link) has no handle; it stays on `paths`.
pub(crate) fn handles_for(paths: &[String]) -> Vec<Json> {
    paths
        .iter()
        .filter_map(|p| {
            let meta = std::fs::metadata(p).ok()?;
            let kind = if meta.is_dir() { crate::fs::Kind::Directory } else { crate::fs::Kind::File };
            Some(crate::fs::handle_json(PathBuf::from(p), kind))
        })
        .collect()
}

struct Boot {
    identity: Identity,
    config: LaunchConfig,
    info: LaunchInfo,
    /// Does THIS process hold the instance lock?
    single_instance: bool,
    /// A dev host's slot — written into a protocol command as `--vn-dist`.
    dev_slot: Option<String>,
}

static BOOT: OnceLock<Boot> = OnceLock::new();

/// The identity `start` resolved (None before it ran).
pub fn identity() -> Option<&'static Identity> {
    BOOT.get().map(|b| &b.identity)
}

/// The launch configuration `start` read (None before it ran).
pub fn config() -> Option<&'static LaunchConfig> {
    BOOT.get().map(|b| &b.config)
}

/// `__vn_launch_info()`'s answer: the boot launch plus who we are. Each call
/// grants fresh `handles` (the runtime asks once per realm, and each realm
/// releases its own through the handles' FinalizationRegistry).
pub fn info_json() -> String {
    match BOOT.get() {
        Some(b) => {
            let mut v = b.info.to_json();
            v["handles"] = Json::Array(handles_for(&b.info.paths));
            v["singleInstance"] = Json::Bool(b.single_instance);
            v["id"] = Json::String(b.identity.id.clone());
            v["name"] = Json::String(b.identity.name.clone());
            v.to_string()
        }
        None => json!({ "argv": [], "cwd": "", "url": null, "paths": [], "handles": [], "singleInstance": false })
            .to_string(),
    }
}

// ---------------------------------------------------------------------------
// the forwarded-launch message (one pipe message, strict JSON)
// ---------------------------------------------------------------------------

/// The cap on one forwarded launch. zed's 128 bytes truncate a long path; a
/// megabyte is a few thousand of them and still bounded.
pub const MAX_MESSAGE: usize = 1 << 20;

/// What a second launch sends the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    pub argv: Vec<String>,
    /// The app arguments (`Args::app_args`) — what `paths` is computed from.
    pub args: Vec<String>,
    pub cwd: String,
    pub url: Option<String>,
}

impl Forward {
    pub fn encode(&self) -> Vec<u8> {
        json!({ "v": 1, "argv": self.argv, "args": self.args, "cwd": self.cwd, "url": self.url })
            .to_string()
            .into_bytes()
    }

    /// STRICT: an object with exactly these keys and types, `v == 1`, at most
    /// [`MAX_MESSAGE`] bytes. Anything else is refused whole — this crosses a
    /// process boundary.
    pub fn decode(bytes: &[u8]) -> Result<Forward, String> {
        if bytes.len() > MAX_MESSAGE {
            return Err(format!("message is {} bytes, over the {MAX_MESSAGE}-byte cap", bytes.len()));
        }
        let v: Json = serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
        let obj = v.as_object().ok_or("not a JSON object")?;
        for k in obj.keys() {
            if !matches!(k.as_str(), "v" | "argv" | "args" | "cwd" | "url") {
                return Err(format!("unknown key {k:?}"));
            }
        }
        if obj.get("v").and_then(Json::as_u64) != Some(1) {
            return Err("`v` must be 1".into());
        }
        let strings = |k: &str| -> Result<Vec<String>, String> {
            obj.get(k)
                .and_then(Json::as_array)
                .ok_or(format!("`{k}` must be an array"))?
                .iter()
                .map(|x| x.as_str().map(str::to_string).ok_or(format!("`{k}` must hold strings")))
                .collect()
        };
        let cwd = obj.get("cwd").and_then(Json::as_str).ok_or("`cwd` must be a string")?.to_string();
        let url = match obj.get("url") {
            None | Some(Json::Null) => None,
            Some(Json::String(s)) => Some(s.clone()),
            Some(_) => return Err("`url` must be a string or null".into()),
        };
        Ok(Forward { argv: strings("argv")?, args: strings("args")?, cwd, url })
    }
}

/// A forwarded launch, delivered: raise the window, then hand it to JS as an
/// unsolicited `launch` event. Runs on the pipe server's thread.
pub(crate) fn deliver(msg: Forward) {
    let paths = existing_paths(&msg.args, Path::new(&msg.cwd));
    if config().is_none_or(|c| c.raise) && !raise_disabled() {
        raise();
    }
    let event = json!({
        "kind": "launch",
        "source": "second-instance",
        "argv": msg.argv,
        "cwd": msg.cwd,
        "url": msg.url,
        "handles": handles_for(&paths),
        "paths": paths,
    });
    crate::emit(event.to_string());
    crate::log_later(
        "launch",
        format!("[launch] a second launch was forwarded here ({} argument(s){})", msg.args.len(), if msg.url.is_some() { ", a URL" } else { "" }),
    );
}

// ---------------------------------------------------------------------------
// raising the primary window
// ---------------------------------------------------------------------------

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// The window a forwarded launch brings to the front. vn-app publishes the
/// primary window's HWND as soon as it exists; a launch that arrives before
/// that has nothing to raise and is still delivered.
pub fn set_window(hwnd: isize) {
    MAIN_HWND.store(hwnd, Ordering::Relaxed);
}

/// `VN_LAUNCH_RAISE=off`: deliver, never take the foreground — the test seam,
/// with VN_DEVTOOLS_RAISE=off as its precedent.
fn raise_disabled() -> bool {
    std::env::var("VN_LAUNCH_RAISE").is_ok_and(|v| v.eq_ignore_ascii_case("off") || v == "0")
}

fn raise() {
    let hwnd = MAIN_HWND.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    #[cfg(target_os = "windows")]
    instance::raise(hwnd);
}

// ---------------------------------------------------------------------------
// start: the first thing main() does
// ---------------------------------------------------------------------------

/// What `main()` should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Run as the app.
    Run,
    /// Exit with this code: 0 = forwarded to the running instance, 2 = the
    /// running instance did not answer in time, 3 = the pipe is not ours.
    Exit(i32),
}

/// How long a second launch waits for the first to answer.
pub const FORWARD_TIMEOUT_MS: u64 = 5_000;

/// Resolves who we are and what this launch is, and — when `singleInstance`
/// is on — either becomes THE instance or forwards to it.
///
/// `build_name` is the build's `name` (`boot_mode::app_name()`); `dev_slot`
/// is the canonical dist slot in a DEV host (it is part of the instance key,
/// so parallel e2e slots of one app never collide), `None` in prod.
pub fn start(args: &Args, config_json: Option<&str>, build_name: Option<String>, dev_slot: Option<PathBuf>) -> Outcome {
    let config = LaunchConfig::parse(config_json);
    let identity = resolve_identity(&config, build_name);
    if !args.ignored.is_empty() {
        // A protocol launch honours nothing but `--vn-dist`: whatever else sits
        // in front of the marker came from a registry command someone edited,
        // or from an attempt to smuggle a mode switch in. Before the logger
        // exists, so stderr now and the log ring once it does.
        let msg = format!(
            "[vue-native compat] a protocol launch ignores host flags in front of --vn-protocol-launch: {}",
            args.ignored.join(" ")
        );
        eprintln!("{msg}");
        crate::log_later("advisory", msg);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let info = LaunchInfo {
        argv: args.argv.clone(),
        cwd: cwd.to_string_lossy().into_owned(),
        url: args.url.clone(),
        paths: existing_paths(&args.app_args, &cwd),
    };
    // `<exe> --vn-unregister-protocols`: the uninstaller's hook. Removes every
    // registration carrying this app's marker and exits, with no window.
    if args.host.unregister_protocols {
        #[cfg(target_os = "windows")]
        for scheme in protocol_win::unregister_all(&protocol_win::Hive::user(), &identity.id) {
            println!("unregistered {scheme}:");
        }
        return Outcome::Exit(0);
    }
    let skip = !config.single_instance
        || args.host.devtools
        || args.host.unregister_protocols
        || args.host.host_info
        || std::env::var("VN_SINGLE_INSTANCE").is_ok_and(|v| v == "0" || v.eq_ignore_ascii_case("off"));
    let mut outcome = Outcome::Run;
    let mut holds = false;
    if !skip {
        #[cfg(target_os = "windows")]
        {
            let fwd = Forward {
                argv: info.argv.clone(),
                args: args.app_args.clone(),
                cwd: info.cwd.clone(),
                url: info.url.clone(),
            };
            match instance::claim(&identity.id, dev_slot.as_deref(), &fwd) {
                instance::Claim::Primary => holds = true,
                instance::Claim::Unavailable(why) => {
                    eprintln!("[vue-native compat] singleInstance is declared but could not be enforced: {why}; running as a separate instance");
                }
                instance::Claim::Forwarded => outcome = Outcome::Exit(0),
                instance::Claim::NoAnswer => {
                    eprintln!(
                        "[launch] {} is already running but did not answer within {} s; not starting a second copy",
                        identity.name,
                        FORWARD_TIMEOUT_MS / 1000
                    );
                    outcome = Outcome::Exit(2);
                }
                instance::Claim::Impostor(why) => {
                    // A-F5.
                    eprintln!(
                        "[vue-native compat] the single-instance pipe for {} is owned by a process that is not this app ({why}) — forwarding refused",
                        identity.name
                    );
                    outcome = Outcome::Exit(3);
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = &dev_slot;
            eprintln!("[vue-native compat] singleInstance is not implemented on this platform yet; running as a separate instance");
        }
    }
    let dev_slot = dev_slot.map(|p| std::path::absolute(&p).unwrap_or(p).to_string_lossy().into_owned());
    let _ = BOOT.set(Boot { identity, config, info, single_instance: holds, dev_slot });
    outcome
}

// ---------------------------------------------------------------------------
// protocols (docs/LAUNCH.md §5)
// ---------------------------------------------------------------------------

fn own_exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The declared `protocols`, at startup: a PACKAGED app (re-)registers each
/// one per-user, idempotently — which also heals a registration after the exe
/// moved (owner decision Q1). A dev host never does so on its own, because
/// that would point the scheme at a dev slot; `appLaunch.registerProtocol()`
/// is the explicit way there (A-F10). Main thread, before the window.
pub fn register_declared() {
    let Some(b) = BOOT.get() else { return };
    if b.config.protocols.is_empty() {
        return;
    }
    if !b.config.single_instance {
        // A-F4.
        crate::advise_later(&format!(
            "\"protocols\" is declared without \"singleInstance\": every {}:// link starts a new instance of the app",
            b.config.protocols[0]
        ));
    }
    if b.dev_slot.is_some() {
        crate::log_later(
            "launch",
            format!(
                "[launch] a dev host does not register the declared protocols ({}); a packaged exe does at startup, \
                 and appLaunch.registerProtocol() does here",
                b.config.protocols.join(", ")
            ),
        );
        return;
    }
    #[cfg(target_os = "windows")]
    {
        if protocol_win::has_package_identity() {
            // A-F9.
            crate::advise_later(
                "this app runs with MSIX package identity: per-user protocol registration is virtualised there and \
                 would register nothing — declare the protocols in AppxManifest (uap:Protocol) instead; skipped",
            );
            return;
        }
        let hive = protocol_win::Hive::user();
        let exe = own_exe();
        for raw in &b.config.protocols {
            let result = protocol::validate_scheme(raw)
                .and_then(|s| protocol_win::register(&hive, &s, &b.identity.id, &exe, None).map(|w| (s, w)));
            match result {
                Ok((s, true)) => crate::log_later("launch", format!("[launch] registered {s}:// → {exe}")),
                Ok((_, false)) => {}
                Err(why) => crate::advise_later(&format!("protocol \"{raw}\" was not registered: {why}")),
            }
        }
    }
}

/// `appLaunch.registerProtocol(scheme)`: `{ok, wrote}` or `{error, name}`.
pub fn js_register(raw: &str) -> String {
    let fail = |m: String| crate::Fail::type_error(format!("registerProtocol: {m}")).to_sync_json();
    let scheme = match protocol::validate_scheme(raw) {
        Ok(s) => s,
        Err(m) => return fail(m),
    };
    let Some(b) = BOOT.get() else { return fail("the launch state is not initialised".into()) };
    #[cfg(target_os = "windows")]
    {
        if protocol_win::has_package_identity() {
            return fail("this app runs with MSIX package identity; declare the protocol in AppxManifest (uap:Protocol)".into());
        }
        if b.dev_slot.is_some() {
            // A-F10.
            crate::advise(&format!(
                "registerProtocol(\"{scheme}\") in a DEV host points {scheme}:// at this dev slot ({}) — a packaged \
                 exe registers its declared protocols itself; unregister this one when done",
                b.dev_slot.as_deref().unwrap_or_default()
            ));
        }
        match protocol_win::register(&protocol_win::Hive::user(), &scheme, &b.identity.id, &own_exe(), b.dev_slot.as_deref()) {
            Ok(wrote) => json!({ "ok": true, "wrote": wrote, "scheme": scheme }).to_string(),
            Err(m) => fail(m),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (b, scheme);
        crate::Fail::new("NotSupportedError", "registerProtocol is not implemented on this platform yet").to_sync_json()
    }
}

/// `appLaunch.unregisterProtocol(scheme)`: true when a registration of OURS,
/// pointing at THIS exe, was removed.
pub fn js_unregister(raw: &str) -> bool {
    let (Ok(scheme), Some(b)) = (protocol::validate_scheme(raw), BOOT.get()) else { return false };
    #[cfg(target_os = "windows")]
    {
        protocol_win::unregister(&protocol_win::Hive::user(), &scheme, &b.identity.id, &own_exe())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (b, scheme);
        false
    }
}

/// `appLaunch.isProtocolRegistered(scheme)`: ours, and pointing at this exe.
pub fn js_is_registered(raw: &str) -> bool {
    let (Ok(scheme), Some(b)) = (protocol::validate_scheme(raw), BOOT.get()) else { return false };
    #[cfg(target_os = "windows")]
    {
        protocol_win::is_registered(&protocol_win::Hive::user(), &scheme, &b.identity.id, &own_exe())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (b, scheme);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_block_parses_with_defaults_and_ignores_wrong_types() {
        assert_eq!(LaunchConfig::parse(None), LaunchConfig::default());
        assert!(LaunchConfig::default().raise, "raising is on by default");
        let c = LaunchConfig::parse(Some(
            r#"{"id":"gl.argon.notes","singleInstance":true,"raiseOnSecondInstance":false,"protocols":["Notes","x-y"]}"#,
        ));
        assert_eq!(c.id.as_deref(), Some("gl.argon.notes"));
        assert!(c.single_instance);
        assert!(!c.raise);
        assert_eq!(c.protocols, vec!["notes", "x-y"]);
        let c = LaunchConfig::parse(Some(r#"{"singleInstance":"yes","protocols":"notes","id":3}"#));
        assert_eq!(c, LaunchConfig::default());
        assert_eq!(LaunchConfig::parse(Some("not json")), LaunchConfig::default());
    }

    #[test]
    fn the_id_defaults_to_the_name() {
        let c = LaunchConfig::default();
        let id = resolve_identity(&c, Some("notes".into()));
        if std::env::var("VN_APP_NAME").is_err() {
            assert_eq!(id, Identity { name: "notes".into(), id: "notes".into() });
        }
        let c = LaunchConfig { id: Some("gl.argon.notes".into()), ..LaunchConfig::default() };
        assert_eq!(resolve_identity(&c, Some("notes".into())).id, "gl.argon.notes");
    }

    #[test]
    fn a_forward_round_trips_and_the_decoder_is_strict() {
        let f = Forward {
            argv: vec!["C:\\a\\Notes.exe".into(), "x.txt".into()],
            args: vec!["x.txt".into()],
            cwd: "C:\\work".into(),
            url: Some("notes://a?b=\"c d\"".into()),
        };
        assert_eq!(Forward::decode(&f.encode()), Ok(f.clone()));
        let none = Forward { url: None, ..f.clone() };
        assert_eq!(Forward::decode(&none.encode()), Ok(none));
        for bad in [
            &br#"[]"#[..],
            br#"{"v":2,"argv":[],"args":[],"cwd":""}"#,
            br#"{"v":1,"argv":[1],"args":[],"cwd":""}"#,
            br#"{"v":1,"argv":[],"args":[],"cwd":"","url":5}"#,
            br#"{"v":1,"argv":[],"args":[],"cwd":"","extra":true}"#,
            br#"{"v":1,"argv":[],"cwd":""}"#,
            b"\xff\xfe",
        ] {
            assert!(Forward::decode(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        let huge = vec![b' '; MAX_MESSAGE + 1];
        assert!(Forward::decode(&huge).unwrap_err().contains("cap"));
    }

    #[test]
    fn paths_are_the_arguments_that_exist() {
        let dir = std::env::temp_dir().join(format!("vn-desktop-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), "x").unwrap();
        let args: Vec<String> =
            ["x.txt", "missing.txt", "--flag", "https://example.com/", ".", dir.join("x.txt").to_str().unwrap()]
                .iter()
                .map(|s| s.to_string())
                .collect();
        let got = existing_paths(&args, &dir);
        let x = dir.join("x.txt").to_string_lossy().into_owned();
        assert_eq!(got, vec![x.clone(), dir.to_string_lossy().into_owned(), x]);
        // The handles name the entry and its kind; the path stays in Rust.
        let missing = dir.join("gone.txt").to_string_lossy().into_owned();
        let hs = handles_for(&[got[0].clone(), got[1].clone(), missing]);
        assert_eq!(hs.len(), 2, "a path that vanished has no handle");
        assert_eq!((hs[0]["name"].as_str(), hs[0]["kind"].as_str()), (Some("x.txt"), Some("file")));
        assert_eq!(hs[1]["kind"].as_str(), Some("directory"));
        for h in &hs {
            assert!(!h.to_string().contains(&*dir.to_string_lossy()), "no path in {h}");
            crate::fs::release(h["h"].as_u64().unwrap());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
