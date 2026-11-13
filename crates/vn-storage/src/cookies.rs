//! The cookie jar's persistence: redjs's `CookieHost`, backed by `<storage folder>/cookies.sealed`.
//!
//! Always sealed, whatever `appStorage`'s `encrypted` setting: a session cookie is the login. Without a
//! provider the jar stays in memory and one advisory says so; it is never written as plaintext.
//!
//! The folder is latched at [`install`]; a later `setStorageFolder` moves `localStorage`, not the jar.
//!
//! redjs calls `store` on every change, from any thread, with its jar lock held, so `store` only stages;
//! [`pump`] seals and writes once per frame and [`flush`] writes now. Lock order: redjs's jar lock, then
//! this module's.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use vn_runtime::redjs::webapi::cookies::{self as jar, Cookie, CookieHost, SameSite};

use crate::{crypt, file};

pub const FILE_NAME: &str = "cookies.sealed";
const FORMAT: &str = "vue-native/cookies";
const SEP: char = '\u{1}';

pub fn path_in(folder: &Path) -> PathBuf {
    folder.join(FILE_NAME)
}

struct State {
    folder: Option<PathBuf>,
    staged: Option<Vec<(String, String)>>,
    count: usize,
    error: Option<String>,
}

pub struct Host {
    state: Mutex<State>,
}

static HOST: Host = Host { state: Mutex::new(State { folder: None, staged: None, count: 0, error: None }) };

impl Host {
    fn note(&self, st: &mut State, what: String) {
        eprintln!("[storage] cookies: {what}");
        st.error = Some(what);
    }

    fn read(&self, st: &mut State) -> Vec<Cookie> {
        let Some(folder) = st.folder.clone() else { return Vec::new() };
        let path = path_in(&folder);
        let Ok(raw) = std::fs::read(&path) else { return Vec::new() };
        let Some(p) = crypt::provider() else {
            self.note(st, format!("{} is sealed and this platform has no decryption provider", path.display()));
            return Vec::new();
        };
        let text = match file::unseal(p, &raw) {
            Ok(t) => t,
            Err(e) => {
                self.note(st, format!("could not decrypt {}: {e} — starting with no cookies", path.display()));
                return Vec::new();
            }
        };
        match file::decode(&text) {
            Ok(entries) => {
                let cookies: Vec<Cookie> = entries.iter().filter_map(|(k, v)| unpack(k, v)).collect();
                if cookies.len() != entries.len() {
                    self.note(st, format!("{} unreadable record(s) in {} skipped", entries.len() - cookies.len(), path.display()));
                }
                st.count = cookies.len();
                cookies
            }
            Err(e) => {
                self.note(st, format!("could not read {}: {e} — starting with no cookies", path.display()));
                Vec::new()
            }
        }
    }

    fn write_staged(&self) -> Result<bool, String> {
        let (folder, entries) = {
            let mut st = self.state.lock().unwrap();
            let Some(entries) = st.staged.take() else { return Ok(false) };
            let Some(folder) = st.folder.clone() else { return Ok(false) };
            (folder, entries)
        };
        let Some(p) = crypt::provider() else {
            return Ok(false);
        };
        let path = path_in(&folder);
        let r = file::seal(p, &file::encode_as(FORMAT, &entries))
            .and_then(|bytes| file::write_atomic(&path, &bytes).map_err(|e| format!("could not write {}: {e}", path.display())));
        let mut st = self.state.lock().unwrap();
        match r {
            Ok(()) => {
                st.error = None;
                Ok(true)
            }
            Err(e) => {
                // Put it back unless something newer was staged, so the next frame retries.
                if st.staged.is_none() {
                    st.staged = Some(entries);
                }
                if st.error.as_deref() != Some(e.as_str()) {
                    eprintln!("[storage] cookies: {e}");
                }
                st.error = Some(e.clone());
                Err(e)
            }
        }
    }
}

impl CookieHost for Host {
    fn load(&self) -> Vec<Cookie> {
        let mut st = self.state.lock().unwrap();
        self.read(&mut st)
    }

    fn store(&self, cookies: &[Cookie]) {
        if crypt::provider().is_none() {
            advise_memory_only();
            return;
        }
        let entries: Vec<(String, String)> = cookies.iter().map(pack).collect();
        let mut st = self.state.lock().unwrap();
        if st.folder.is_none() {
            return;
        }
        st.count = entries.len();
        st.staged = Some(entries);
    }

    fn describe(&self) -> String {
        match &self.state.lock().unwrap().folder {
            Some(f) => path_in(f).display().to_string(),
            None => String::from("(no folder)"),
        }
    }
}

fn advise_memory_only() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        eprintln!(
            "[storage] cookies: this platform has no encryption provider, and cookies are credentials, \
             so they are never written in plaintext — the jar is IN MEMORY ONLY and signing in does \
             not survive a restart"
        );
    });
}

fn pack(c: &Cookie) -> (String, String) {
    let b = |v: bool| if v { "1" } else { "0" };
    let record = [
        c.domain.as_str(),
        c.path.as_str(),
        b(c.host_only),
        b(c.secure),
        b(c.http_only),
        c.same_site.as_str(),
        &c.expires.map(|e| e.to_string()).unwrap_or_default(),
        &c.creation.to_string(),
        &c.last_access.to_string(),
        c.value.as_str(),
    ]
    .join(&SEP.to_string());
    (c.name.clone(), record)
}

fn unpack(name: &str, record: &str) -> Option<Cookie> {
    let f: Vec<&str> = record.splitn(10, SEP).collect();
    if f.len() != 10 {
        return None;
    }
    let flag = |s: &str| match s {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    };
    Some(Cookie {
        name: name.to_string(),
        value: f[9].to_string(),
        domain: f[0].to_string(),
        path: f[1].to_string(),
        host_only: flag(f[2])?,
        secure: flag(f[3])?,
        http_only: flag(f[4])?,
        same_site: SameSite::parse(f[5]),
        expires: Some(f[6].parse().ok()?),
        creation: f[7].parse().ok()?,
        last_access: f[8].parse().ok()?,
    })
}

pub fn install(folder: PathBuf) {
    HOST.state.lock().unwrap().folder = Some(folder);
    jar::set_host(&HOST);
    let _ = jar::get(None);
}

pub fn folder() -> Option<PathBuf> {
    HOST.state.lock().unwrap().folder.clone()
}

pub fn pump() {
    let _ = HOST.write_staged();
}

pub fn flush() -> Result<bool, String> {
    HOST.write_staged()
}

pub fn info_json() -> String {
    let st = HOST.state.lock().unwrap();
    let file = st.folder.as_deref().map(path_in);
    let bytes = file.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0);
    format!(
        "{{\"file\":{},\"sealed\":true,\"bytes\":{},\"persisted\":{},\"pending\":{},\"error\":{}}}",
        match &file {
            Some(p) => format!("\"{}\"", crate::json_escape(&p.display().to_string())),
            None => String::from("null"),
        },
        bytes,
        st.count,
        st.staged.is_some(),
        match &st.error {
            Some(e) => format!("\"{}\"", crate::json_escape(e)),
            None => String::from("null"),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vn-storage-cookies-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn host_in(folder: &Path) -> Host {
        Host {
            state: Mutex::new(State { folder: Some(folder.to_path_buf()), staged: None, count: 0, error: None }),
        }
    }

    fn sample() -> Vec<Cookie> {
        vec![
            Cookie {
                name: "sid".into(),
                value: "s3cret-value".into(),
                domain: "argon.gl".into(),
                path: "/".into(),
                host_only: false,
                secure: true,
                http_only: true,
                same_site: SameSite::None,
                expires: Some(4_000_000_000),
                creation: 1_700_000_000_000_001,
                last_access: 1_700_000_000_000_002,
            },
            Cookie {
                name: String::new(),
                value: "nameless=has an equals; and spaces".into(),
                domain: "localhost".into(),
                path: "/a/b".into(),
                host_only: true,
                secure: false,
                http_only: false,
                same_site: SameSite::Unspecified,
                expires: Some(1),
                creation: 3,
                last_access: 4,
            },
        ]
    }

    #[test]
    fn a_record_round_trips_every_field() {
        for c in sample() {
            let (k, v) = pack(&c);
            assert_eq!(unpack(&k, &v), Some(c));
        }
        assert_eq!(unpack("x", "too\u{1}few"), None);
        assert_eq!(unpack("x", "d\u{1}/\u{1}2\u{1}0\u{1}0\u{1}\u{1}1\u{1}2\u{1}3\u{1}v"), None, "a bad flag");
        assert_eq!(unpack("x", "d\u{1}/\u{1}1\u{1}0\u{1}0\u{1}\u{1}\u{1}2\u{1}3\u{1}v"), None, "no expiry");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn store_stages_pump_seals_and_a_fresh_host_loads_the_same_jar() {
        let dir = temp("round");
        let h = host_in(&dir);
        h.store(&sample());
        assert!(!path_in(&dir).exists(), "store only stages");
        assert_eq!(h.write_staged(), Ok(true));
        assert_eq!(h.write_staged(), Ok(false), "one write per staged change");
        let raw = std::fs::read(path_in(&dir)).unwrap();
        for needle in ["s3cret-value", "sid", "argon.gl", "entries"] {
            assert!(!raw.windows(needle.len()).any(|w| w == needle.as_bytes()), "{needle:?} is legible");
        }
        assert_eq!(host_in(&dir).load(), sample());
        h.store(&sample()[..1]);
        h.store(&sample()[1..]);
        h.write_staged().unwrap();
        assert_eq!(host_in(&dir).load(), sample()[1..].to_vec());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn an_unreadable_file_is_an_empty_jar_and_an_error_not_a_failure() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path_in(&dir), b"not sealed at all").unwrap();
        let h = host_in(&dir);
        assert!(h.load().is_empty());
        assert!(h.state.lock().unwrap().error.as_deref().unwrap().contains("could not decrypt"));
        h.store(&sample());
        h.write_staged().unwrap();
        assert_eq!(host_in(&dir).load(), sample());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_file_is_an_empty_jar_and_no_folder_stages_nothing() {
        let dir = temp("none");
        assert!(host_in(&dir).load().is_empty());
        let h = Host { state: Mutex::new(State { folder: None, staged: None, count: 0, error: None }) };
        h.store(&sample());
        assert_eq!(h.write_staged(), Ok(false));
    }
}
