//! What the user answered to this app's own permission prompt, per app, on
//! disk (docs/PERMISSIONS.md §5.1).
//!
//! A browser remembers "Allow" and "Block" per site. An app runs as one
//! "site", so the answers live in the app's storage folder (the one
//! `localStorage` uses, `%LOCALAPPDATA%\<app>\storage`), in
//! `permissions.json`:
//!
//! ```json
//! { "version": 1,
//!   "permissions": { "microphone": { "decision": "allow", "at": 1793568000000 } } }
//! ```
//!
//! `at` is when it was answered (Unix ms). Nothing else is kept. A dismissal
//! (Esc) is not a decision and is never written.
//!
//! The file is read once, at the first question, and written through on
//! every change: a decision must outlive a crash. It is written to a
//! temporary file first, then renamed over the old one. A host with no
//! storage folder (built without `storages`) keeps the answers for the run
//! only, as a private browser window does. A file that cannot be read is
//! left alone until the next answer replaces it, and the reason is logged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value as Json};
use vn_runtime::redjs::webapi::permissions::Capability;

/// The user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Block => "block",
        }
    }

    fn parse(s: &str) -> Option<Decision> {
        match s {
            "allow" => Some(Decision::Allow),
            "block" => Some(Decision::Block),
            _ => None,
        }
    }
}

/// One stored answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub decision: Decision,
    /// Unix ms.
    pub at: u64,
}

/// The decisions of one app: in memory, and in a file when there is one.
#[derive(Default)]
pub struct Store {
    path: Option<PathBuf>,
    loaded: bool,
    map: BTreeMap<&'static str, Entry>,
}

impl Store {
    pub const fn new() -> Store {
        Store { path: None, loaded: false, map: BTreeMap::new() }
    }

    /// Where the decisions live from now on (`None`: memory only). Read
    /// again at the next question.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
        self.loaded = false;
        self.map.clear();
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(path) = &self.path else { return };
        match std::fs::read_to_string(path) {
            Ok(text) => match parse(&text) {
                Ok(map) => self.map = map,
                Err(e) => eprintln!("[permissions] {} is not readable ({e}); it is replaced at the next answer", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("[permissions] could not read {}: {e}", path.display()),
        }
    }

    pub fn get(&mut self, cap: Capability) -> Option<Entry> {
        self.load();
        self.map.get(cap.web_name()).copied()
    }

    /// Stores (`Some`) or forgets (`None`) the answer for `cap`. `true` when
    /// that changed anything. A write that fails is logged; the answer
    /// stands for this run either way.
    pub fn set(&mut self, cap: Capability, decision: Option<Decision>) -> bool {
        self.load();
        let before = self.map.get(cap.web_name()).map(|e| e.decision);
        if before == decision {
            return false;
        }
        match decision {
            Some(d) => {
                self.map.insert(cap.web_name(), Entry { decision: d, at: now_ms() });
            }
            None => {
                self.map.remove(cap.web_name());
            }
        }
        if let Some(path) = &self.path {
            if let Err(e) = write(path, &encode(&self.map)) {
                eprintln!("[permissions] could not write {}: {e} (the answer holds for this run)", path.display());
            }
        }
        true
    }

    /// Every stored answer, in name order.
    pub fn list(&mut self) -> Vec<(Capability, Entry)> {
        self.load();
        self.map.iter().filter_map(|(name, e)| Capability::from_web_name(name).map(|c| (c, *e))).collect()
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn encode(map: &BTreeMap<&'static str, Entry>) -> String {
    let mut permissions = serde_json::Map::new();
    for (name, e) in map {
        permissions.insert((*name).to_string(), json!({ "decision": e.decision.as_str(), "at": e.at }));
    }
    serde_json::to_string_pretty(&json!({ "version": 1, "permissions": permissions })).unwrap_or_default()
}

/// The file's text → its answers. Names this build does not know and
/// entries it cannot read are skipped: a newer build's file still works.
fn parse(text: &str) -> Result<BTreeMap<&'static str, Entry>, String> {
    let v: Json = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Some(perms) = v.get("permissions").and_then(Json::as_object) else {
        return Err("no `permissions` object".into());
    };
    let mut map = BTreeMap::new();
    for (name, e) in perms {
        let Some(cap) = Capability::from_web_name(name) else { continue };
        let Some(decision) = e.get("decision").and_then(Json::as_str).and_then(Decision::parse) else { continue };
        let at = e.get("at").and_then(Json::as_u64).unwrap_or(0);
        map.insert(cap.web_name(), Entry { decision, at });
    }
    Ok(map)
}

/// Write-then-rename, so a crash mid-write leaves the old file whole.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Scratch {
            let p = std::env::temp_dir().join(format!("vn-permissions-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Scratch(p)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_answer_survives_a_restart_and_a_reset_forgets_it() {
        let dir = Scratch::new("restart");
        let file = dir.0.join("storage").join("permissions.json");
        let mut s = Store::new();
        s.set_path(Some(file.clone()));
        assert_eq!(s.get(Capability::Microphone), None);
        assert!(s.set(Capability::Microphone, Some(Decision::Block)));
        assert!(!s.set(Capability::Microphone, Some(Decision::Block)), "the same answer is not a change");
        assert!(file.is_file(), "written through, creating the folder");
        // Another run of the app.
        let mut again = Store::new();
        again.set_path(Some(file.clone()));
        let e = again.get(Capability::Microphone).unwrap();
        assert_eq!(e.decision, Decision::Block);
        assert!(e.at > 1_700_000_000_000, "{e:?}");
        assert!(again.set(Capability::Microphone, Some(Decision::Allow)));
        assert!(again.set(Capability::Microphone, None));
        let mut third = Store::new();
        third.set_path(Some(file.clone()));
        assert_eq!(third.get(Capability::Microphone), None);
        assert!(!file.with_extension("json.tmp").exists(), "no temporary file is left");
    }

    #[test]
    fn the_file_is_versioned_json_with_web_names() {
        let mut map = BTreeMap::new();
        map.insert("microphone", Entry { decision: Decision::Allow, at: 5 });
        let text = encode(&map);
        let v: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["permissions"]["microphone"], json!({ "decision": "allow", "at": 5 }));
        assert_eq!(parse(&text).unwrap(), map);
    }

    #[test]
    fn what_it_cannot_read_it_skips_and_a_broken_file_reads_as_empty() {
        let text = r#"{"version":2,"permissions":{"microphone":{"decision":"block"},"midi":{"decision":"allow"},
                       "camera":{"decision":"maybe"}}}"#;
        let map = parse(text).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map["microphone"], Entry { decision: Decision::Block, at: 0 });
        assert!(parse("not json").is_err());
        assert!(parse("{}").is_err());
        let dir = Scratch::new("broken");
        std::fs::create_dir_all(&dir.0).unwrap();
        let file = dir.0.join("permissions.json");
        std::fs::write(&file, "{ broken").unwrap();
        let mut s = Store::new();
        s.set_path(Some(file.clone()));
        assert_eq!(s.get(Capability::Microphone), None);
        assert!(s.set(Capability::Microphone, Some(Decision::Allow)));
        assert!(parse(&std::fs::read_to_string(&file).unwrap()).is_ok(), "the next answer replaces it");
    }

    #[test]
    fn without_a_folder_the_answers_last_for_the_run() {
        let mut s = Store::new();
        assert!(s.path().is_none());
        assert!(s.set(Capability::Microphone, Some(Decision::Allow)));
        assert_eq!(s.get(Capability::Microphone).map(|e| e.decision), Some(Decision::Allow));
        assert_eq!(s.list().len(), 1);
    }
}
