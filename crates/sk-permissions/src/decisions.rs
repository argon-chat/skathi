//! The user's answers to this app's own permission prompt, kept in `permissions.json` in the app's
//! storage folder (`{"version":1,"permissions":{"microphone":{"decision":"allow","at":<unix ms>}}}`).
//! A dismissal is never written.
//!
//! Read once at the first question, written through on every change (temp file, then rename). A file
//! that is not JSON is left until the next answer replaces it; one that could not be read (held open
//! elsewhere) is read again at the next question and never written over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value as Json};
use sk_runtime::redjs::webapi::permissions::Capability;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub decision: Decision,
    pub at: u64,
}

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

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
        self.loaded = false;
        self.map.clear();
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Reads the file once. A failed read leaves the store unloaded and nothing is written meanwhile, so
    /// an answer cannot replace a file whose other answers were never seen.
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        let Some(path) = &self.path else {
            self.loaded = true;
            return;
        };
        match read(path) {
            Ok(text) => {
                self.loaded = true;
                match parse(&text) {
                    Ok(map) => {
                        let newer = std::mem::replace(&mut self.map, map);
                        self.map.extend(newer);
                    }
                    Err(e) => eprintln!("[permissions] {} is not readable ({e}); it is replaced at the next answer", path.display()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.loaded = true,
            Err(e) => eprintln!("[permissions] could not read {}: {e} (read again at the next question)", path.display()),
        }
    }

    pub fn get(&mut self, cap: Capability) -> Option<Entry> {
        self.load();
        self.map.get(cap.web_name()).copied()
    }

    /// Stores (`Some`) or forgets (`None`) the answer for `cap`; `true` when that changed anything.
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
            if !self.loaded {
                // Unreadable file: writing now would drop every answer in it but this one.
                eprintln!("[permissions] {} was not read; the answer holds for this run and is not written", path.display());
            } else if let Err(e) = write(path, &encode(&self.map)) {
                eprintln!("[permissions] could not write {}: {e} (the answer holds for this run)", path.display());
            }
        }
        true
    }

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

/// Unknown names and unreadable entries are skipped: a newer build's file still works.
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

#[cfg(not(test))]
fn read(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

#[cfg(test)]
fn read(path: &Path) -> std::io::Result<String> {
    if tests::FAIL_READS.with(|f| f.get() > 0) {
        tests::FAIL_READS.with(|f| f.set(f.get() - 1));
        return Err(std::io::Error::other("the process cannot access the file because it is being used by another process"));
    }
    std::fs::read_to_string(path)
}

/// Write-then-rename, so a crash mid-write leaves the old file whole.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}-{seq}.tmp", std::process::id()));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    })();
    let renamed = written.and_then(|()| std::fs::rename(&tmp, path));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        pub(super) static FAIL_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Scratch {
            let p = std::env::temp_dir().join(format!("sk-permissions-{name}-{}", std::process::id()));
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
        let left: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["permissions.json".to_string()], "no temporary file is left");
    }

    #[test]
    fn a_file_that_could_not_be_read_is_neither_lost_nor_overwritten() {
        let dir = Scratch::new("transient");
        let file = dir.0.join("permissions.json");
        let mut s = Store::new();
        s.set_path(Some(file.clone()));
        assert!(s.set(Capability::Microphone, Some(Decision::Block)));
        assert!(s.set(Capability::Camera, Some(Decision::Allow)));
        let mut again = Store::new();
        again.set_path(Some(file.clone()));
        FAIL_READS.with(|f| f.set(2));
        assert_eq!(again.get(Capability::Microphone), None, "unread: nothing known yet");
        assert!(again.set(Capability::Geolocation, Some(Decision::Allow)));
        let on_disk = parse(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(on_disk.len(), 2, "the camera's and the microphone's answers survive: {on_disk:?}");
        assert_eq!(again.get(Capability::Microphone).map(|e| e.decision), Some(Decision::Block));
        assert_eq!(again.get(Capability::Camera).map(|e| e.decision), Some(Decision::Allow));
        assert_eq!(again.get(Capability::Geolocation).map(|e| e.decision), Some(Decision::Allow), "and this run's answer stays");
        assert!(again.set(Capability::Microphone, Some(Decision::Allow)));
        let on_disk = parse(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(on_disk["microphone"].decision, Decision::Allow);
        assert_eq!(on_disk["camera"].decision, Decision::Allow, "kept once read");
        assert_eq!(on_disk["geolocation"].decision, Decision::Allow, "written with the rest");
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
