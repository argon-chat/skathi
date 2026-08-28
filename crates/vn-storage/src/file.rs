//! The on-disk shape of a storage area, and the discipline that writes it.
//!
//! # One file per area, not one file per key
//!
//! A key/value pair is not the unit anything cares about — the *area* is. Web
//! Storage's own operations (`clear`, enumeration order, the quota) are
//! area-wide, an area is the thing a `setItem` has to leave consistent, and the
//! only file update that is crash-safe is one that replaces a whole file by
//! rename. Per-key files would give up all of that (a `clear` becomes N
//! unlinks that can be interrupted halfway) and add one that is worse: **a key
//! name would become a file name**, so `localStorage.setItem("auth-token", …)`
//! would leave the string `auth-token` legible in a directory listing of an
//! otherwise encrypted store, and keys containing `/`, `:` or a reserved
//! Windows device name would need an escaping scheme nobody would ever read.
//!
//! So: one file, whole-area writes, `write-to-.tmp` then `rename` over the
//! real name — the same discipline `packages/vue-native/build.ts` uses for
//! `manifest.json`, for the same reason. `rename` on Windows is `MoveFileEx`
//! with `MOVEFILE_REPLACE_EXISTING`, which is atomic against a reader.
//!
//! The temporary file carries the **process id**, so two hosts writing the same
//! folder cannot land on one another's scratch file. They can still overwrite
//! one another's *area* — last writer wins, documented in
//! [`crate`]'s header — but neither of them can ever see a half-written one.
//!
//! # Two names, one format
//!
//! `localStorage.json` is the plaintext file: real JSON, indented one entry per
//! line, the sort of thing a person can open when they are trying to work out
//! what an app remembered about them.
//!
//! `localStorage.sealed` is the encrypted one: an 8-byte header
//! (`VNSTOR`, format version, provider id) followed by the ciphertext of
//! **exactly the same JSON**. Whole-file rather than per-value, for three
//! reasons: the key names are hidden too (a per-value scheme leaks the shape of
//! the data), it is one `CryptProtectData` call instead of one per key, and a
//! file that is either entirely readable or entirely not has no partially
//! decryptable state to reason about.
//!
//! The provider id in the header is what makes a file written by one provider
//! *refuse* to open under another, instead of failing somewhere deeper with a
//! worse message.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::crypt::SecretProvider;

pub const PLAIN_NAME: &str = "localStorage.json";
pub const SEALED_NAME: &str = "localStorage.sealed";

const MAGIC: &[u8; 6] = b"VNSTOR";
const FORMAT_VERSION: u8 = 1;
/// `MAGIC` + format version + provider id.
const HEADER_LEN: usize = 8;

pub fn plain_path(folder: &Path) -> PathBuf {
    folder.join(PLAIN_NAME)
}

pub fn sealed_path(folder: &Path) -> PathBuf {
    folder.join(SEALED_NAME)
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// The area, as the bytes that go into the file (plaintext or, once sealed,
/// ciphertext).
pub fn encode(entries: &[(String, String)]) -> String {
    let mut s = String::from("{\n  \"format\": \"vue-native/storage\",\n  \"version\": 1,\n  \"entries\": [");
    for (i, (k, v)) in entries.iter().enumerate() {
        s.push_str(if i == 0 { "\n    " } else { ",\n    " });
        s.push('[');
        write_json_string(&mut s, k);
        s.push_str(", ");
        write_json_string(&mut s, v);
        s.push(']');
    }
    if !entries.is_empty() {
        s.push_str("\n  ");
    }
    s.push_str("]\n}\n");
    s
}

fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Reads what [`encode`] wrote.
///
/// A deliberately small parser rather than a general JSON one: this file has
/// exactly one shape, it is written by this module and nothing else, and the
/// interesting half — string unescaping — is the same either way. Unknown
/// members are skipped so a future version can add one without orphaning
/// today's files.
pub fn decode(text: &str) -> Result<Vec<(String, String)>, String> {
    let b = text.as_bytes();
    let mut p = Parser { b, i: 0 };
    p.ws();
    p.expect(b'{')?;
    let mut entries: Option<Vec<(String, String)>> = None;
    loop {
        p.ws();
        if p.eat(b'}') {
            break;
        }
        let name = p.string()?;
        p.ws();
        p.expect(b':')?;
        p.ws();
        if name == "entries" {
            entries = Some(p.entries()?);
        } else {
            p.skip_value()?;
        }
        p.ws();
        if !p.eat(b',') {
            p.ws();
            p.expect(b'}')?;
            break;
        }
    }
    entries.ok_or_else(|| String::from("no \"entries\" member"))
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.i < self.b.len() && self.b[self.i] == c {
            self.i += 1;
            return true;
        }
        false
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.eat(c) {
            return Ok(());
        }
        Err(format!(
            "expected {:?} at byte {}, found {:?}",
            c as char,
            self.i,
            self.b.get(self.i).map(|&c| c as char)
        ))
    }

    fn entries(&mut self) -> Result<Vec<(String, String)>, String> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.ws();
            if self.eat(b']') {
                return Ok(out);
            }
            self.expect(b'[')?;
            self.ws();
            let k = self.string()?;
            self.ws();
            self.expect(b',')?;
            self.ws();
            let v = self.string()?;
            self.ws();
            self.expect(b']')?;
            out.push((k, v));
            self.ws();
            if !self.eat(b',') {
                self.ws();
                self.expect(b']')?;
                return Ok(out);
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(&c) = self.b.get(self.i) else {
                return Err(String::from("unterminated string"));
            };
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(&e) = self.b.get(self.i) else {
                        return Err(String::from("unterminated escape"));
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            // A surrogate pair is two escapes; anything else
                            // (including a lone surrogate, which we never
                            // write) becomes U+FFFD rather than an error —
                            // one bad character must not cost the whole area.
                            let ch = if (0xd800..0xdc00).contains(&hi)
                                && self.b.get(self.i) == Some(&b'\\')
                                && self.b.get(self.i + 1) == Some(&b'u')
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                char::from_u32(
                                    0x10000 + ((hi as u32 - 0xd800) << 10) + (lo as u32 - 0xdc00),
                                )
                                .unwrap_or('\u{fffd}')
                            } else {
                                char::from_u32(hi as u32).unwrap_or('\u{fffd}')
                            };
                            out.push(ch);
                        }
                        other => return Err(format!("unknown escape \\{}", other as char)),
                    }
                }
                _ => {
                    // Multi-byte UTF-8 passes through untouched: find the end
                    // of this character in the original str.
                    let start = self.i - 1;
                    while self.i < self.b.len() && (self.b[self.i] & 0xc0) == 0x80 {
                        self.i += 1;
                    }
                    out.push_str(
                        std::str::from_utf8(&self.b[start..self.i]).map_err(|e| e.to_string())?,
                    );
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let end = self.i + 4;
        let s = self
            .b
            .get(self.i..end)
            .ok_or_else(|| String::from("truncated \\u escape"))?;
        let s = std::str::from_utf8(s).map_err(|e| e.to_string())?;
        let v = u16::from_str_radix(s, 16).map_err(|e| e.to_string())?;
        self.i = end;
        Ok(v)
    }

    /// Skips one value of any type — how an unknown member is tolerated.
    fn skip_value(&mut self) -> Result<(), String> {
        self.ws();
        match self.b.get(self.i) {
            Some(b'"') => {
                self.string()?;
                Ok(())
            }
            Some(b'[') | Some(b'{') => {
                let (open, close) = if self.b[self.i] == b'[' { (b'[', b']') } else { (b'{', b'}') };
                let mut depth = 0usize;
                while let Some(&c) = self.b.get(self.i) {
                    match c {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        c if c == open => depth += 1,
                        c if c == close => {
                            depth -= 1;
                            self.i += 1;
                            if depth == 0 {
                                return Ok(());
                            }
                            continue;
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                Err(String::from("unterminated array or object"))
            }
            Some(_) => {
                while let Some(&c) = self.b.get(self.i) {
                    if c == b',' || c == b'}' || c == b']' {
                        break;
                    }
                    self.i += 1;
                }
                Ok(())
            }
            None => Err(String::from("value expected")),
        }
    }
}

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

pub fn seal(p: &dyn SecretProvider, json: &str) -> Result<Vec<u8>, String> {
    let body = p.protect(json.as_bytes())?;
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.push(p.id());
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn unseal(p: &dyn SecretProvider, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() < HEADER_LEN || &bytes[..6] != MAGIC {
        return Err(format!("not a {SEALED_NAME} file (bad magic)"));
    }
    if bytes[6] != FORMAT_VERSION {
        return Err(format!(
            "storage file format version {} — this build writes version {FORMAT_VERSION}",
            bytes[6]
        ));
    }
    if bytes[7] != p.id() {
        return Err(format!(
            "this file was sealed by provider {} and this build has {} ({})",
            bytes[7],
            p.id(),
            p.name()
        ));
    }
    let plain = p.unprotect(&bytes[HEADER_LEN..])?;
    String::from_utf8(plain).map_err(|e| format!("decrypted bytes are not UTF-8: {e}"))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Write, then rename over the target. Never leaves a partial file where the
/// real one was, and never collides with another process's scratch file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        // The rename is atomic; the CONTENT reaching the platter is not, and a
        // machine that loses power between the two would rename an empty file
        // over a good one.
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(entries: Vec<(String, String)>) {
        let text = encode(&entries);
        let back = decode(&text).unwrap_or_else(|e| panic!("{e}\n--- file ---\n{text}"));
        assert_eq!(back, entries);
    }

    #[test]
    fn an_empty_area_round_trips_and_is_still_valid_json_shaped() {
        let text = encode(&[]);
        assert!(text.contains("\"entries\": []"), "{text}");
        assert_eq!(decode(&text).unwrap(), Vec::<(String, String)>::new());
    }

    #[test]
    fn ordinary_entries_round_trip_in_order() {
        round_trip(vec![
            ("b".into(), "2".into()),
            ("a".into(), "1".into()),
            ("c".into(), "3".into()),
        ]);
    }

    #[test]
    fn every_character_a_value_can_contain_survives() {
        // Web Storage values are arbitrary strings: quotes, backslashes,
        // newlines and control characters all occur in real data (a serialized
        // JSON blob stored as a value is the common case).
        round_trip(vec![
            ("quote\"key".into(), "back\\slash".into()),
            ("nl".into(), "line1\nline2\r\n".into()),
            ("tab".into(), "a\tb".into()),
            ("ctrl".into(), "\u{1}\u{1f}".into()),
            ("unicode".into(), "привет 😀 \u{fffd}".into()),
            ("empty".into(), String::new()),
            (String::new(), "empty key".into()),
            ("json".into(), r#"{"nested":["a","b"],"n":1}"#.into()),
        ]);
    }

    #[test]
    fn a_json_blob_stored_as_a_value_comes_back_byte_identical() {
        // The single most common real value, and the one an escaping bug
        // corrupts silently.
        let payload = r#"{"user":{"name":"O'Brien \"Bob\"","roles":["a\\b"]}}"#;
        round_trip(vec![("session".into(), payload.into())]);
    }

    #[test]
    fn an_unknown_member_is_skipped_rather_than_fatal() {
        // Forward compatibility: a file written by a later build that added a
        // member must still load here, or an upgrade-then-downgrade loses data.
        let text = r#"{"format":"vue-native/storage","version":1,
            "writtenBy":{"app":"hello","pid":123},"tags":[1,2,3],"encrypted":false,
            "entries":[["k","v"]]}"#;
        assert_eq!(decode(text).unwrap(), vec![("k".to_string(), "v".to_string())]);
    }

    #[test]
    fn entries_may_come_before_other_members() {
        let text = r#"{"entries":[["a","1"]],"version":1}"#;
        assert_eq!(decode(text).unwrap(), vec![("a".to_string(), "1".to_string())]);
    }

    #[test]
    fn a_truncated_or_foreign_file_is_an_error_not_an_empty_area() {
        // The difference between "this app has no saved data" and "this app's
        // saved data could not be read" has to survive to the caller, which is
        // what makes the advisory in lib.rs possible.
        for bad in [
            "",
            "{",
            "{\"entries\":",
            "{\"entries\":[[\"a\"]]}",
            "{\"entries\":[[\"a\",\"b\"}]}",
            "{\"version\":1}",
            "not json at all",
        ] {
            assert!(decode(bad).is_err(), "{bad:?} must not parse");
        }
    }

    #[test]
    fn escapes_this_module_never_writes_are_still_read() {
        // Someone WILL hand-edit this file. `\/`, `A` and a surrogate
        // pair are all legal JSON and all mean something.
        let text = r#"{"entries":[["a\/b","Aé😀"]]}"#;
        assert_eq!(
            decode(text).unwrap(),
            vec![("a/b".to_string(), "Aé😀".to_string())]
        );
    }

    #[test]
    fn write_atomic_replaces_an_existing_file_and_leaves_no_scratch_behind() {
        let dir = std::env::temp_dir().join(format!("vn-storage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("area.json");
        write_atomic(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "scratch files left behind: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn a_sealed_file_carries_a_header_and_does_not_contain_the_plaintext() {
        let p = crate::crypt::provider().unwrap();
        let json = encode(&[("token".into(), "hunter2".into())]);
        let sealed = seal(p, &json).unwrap();
        assert_eq!(&sealed[..6], MAGIC);
        assert_eq!(sealed[6], FORMAT_VERSION);
        assert_eq!(sealed[7], p.id());
        for needle in [b"hunter2".as_slice(), b"token".as_slice(), b"entries".as_slice()] {
            assert!(
                !sealed.windows(needle.len()).any(|w| w == needle),
                "{:?} is legible in the sealed file",
                std::str::from_utf8(needle).unwrap()
            );
        }
        assert_eq!(unseal(p, &sealed).unwrap(), json);
        assert_eq!(decode(&unseal(p, &sealed).unwrap()).unwrap(), vec![("token".to_string(), "hunter2".to_string())]);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn unsealing_something_that_is_not_ours_says_which_check_failed() {
        let p = crate::crypt::provider().unwrap();
        assert!(unseal(p, b"").unwrap_err().contains("bad magic"));
        assert!(unseal(p, b"not-a-sealed-file").unwrap_err().contains("bad magic"));
        let mut wrong_version = seal(p, "{}").unwrap();
        wrong_version[6] = 9;
        assert!(unseal(p, &wrong_version).unwrap_err().contains("version 9"));
        let mut wrong_provider = seal(p, "{}").unwrap();
        wrong_provider[7] = 7;
        assert!(unseal(p, &wrong_provider).unwrap_err().contains("provider 7"));
    }
}
