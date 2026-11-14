//! Custom URL protocols: the rules, independent of storage (`protocol_win.rs` is the registry).

use crate::args::quote_always;

const RESERVED: &[&str] = &[
    "http", "https", "file", "ftp", "mailto", "tel", "data", "blob", "javascript", "about", "vbscript", "search-ms",
];

/// A scheme as the registry will hold it, or why not: `[a-z][a-z0-9+.-]{0,62}` lowercased, none of [`RESERVED`].
pub fn validate_scheme(raw: &str) -> Result<String, String> {
    let s = raw.trim().trim_end_matches('/').trim_end_matches(':').to_ascii_lowercase();
    let mut chars = s.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));
    if !ok {
        return Err(format!(
            "{raw:?} is not a URL scheme (a letter, then up to 62 letters, digits, \"+\", \"-\" or \".\")"
        ));
    }
    if RESERVED.contains(&s.as_str()) || s.starts_with("ms-") {
        return Err(format!("\"{s}\" belongs to the browser or to Windows and cannot be registered by an app"));
    }
    Ok(s)
}

/// The `shell\open\command` a registration writes. The shell substitutes `%1` verbatim, so the URL is
/// recovered from the raw command line after the marker's `--` rather than re-split.
pub fn command_line(exe: &str, dev_slot: Option<&str>) -> String {
    let mut out = quote_always(exe);
    if let Some(slot) = dev_slot {
        out.push_str(" --sk-dist ");
        out.push_str(&quote_always(slot));
    }
    out.push_str(" --sk-protocol-launch -- \"%1\"");
    out
}

pub fn command_targets(command: &str, exe: &str) -> bool {
    let want = quote_always(exe);
    command.len() >= want.len() && command[..want.len()].eq_ignore_ascii_case(&want)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{parse, split_command_line};

    #[test]
    fn schemes_are_validated_and_lowercased() {
        assert_eq!(validate_scheme("Notes").as_deref(), Ok("notes"));
        assert_eq!(validate_scheme("notes://").as_deref(), Ok("notes"));
        assert_eq!(validate_scheme("web+notes").as_deref(), Ok("web+notes"));
        assert_eq!(validate_scheme("x-y.z").as_deref(), Ok("x-y.z"));
        for bad in ["", "1notes", "no tes", "nötes", "a/b", &"a".repeat(64)] {
            assert!(validate_scheme(bad).is_err(), "{bad:?}");
        }
        for reserved in ["http", "HTTPS", "file", "mailto", "javascript", "ms-settings", "ms-msdt", "search-ms"] {
            assert!(validate_scheme(reserved).unwrap_err().contains("cannot be registered"), "{reserved}");
        }
    }

    #[test]
    fn the_command_line_quotes_every_path_and_ends_in_a_quoted_percent_one() {
        assert_eq!(
            command_line(r"C:\Program Files\Notes\Notes.exe", None),
            r#""C:\Program Files\Notes\Notes.exe" --sk-protocol-launch -- "%1""#
        );
        assert_eq!(
            command_line(r"C:\sk\sk-app.exe", Some(r"C:\slots\a b\")),
            r#""C:\sk\sk-app.exe" --sk-dist "C:\slots\a b\\" --sk-protocol-launch -- "%1""#
        );
    }

    #[test]
    fn what_the_shell_builds_from_it_parses_back_byte_exact() {
        let url = r#"notes://open?b="c d"&e=%22 --devtools"#;
        for (exe, slot) in [
            (r"C:\Program Files\Notes\Notes.exe", None),
            (r"C:\sk\sk-app.exe", Some(r"C:\slots\one\")),
        ] {
            let raw = command_line(exe, slot).replace("%1", url);
            let argv: Vec<String> = split_command_line(&raw).into_iter().map(|t| t.text).collect();
            let a = parse(&argv, Some(&raw));
            assert!(a.protocol_launch);
            assert_eq!(a.url.as_deref(), Some(url));
            assert_eq!(a.exe, exe);
            assert_eq!(a.host.sk_dist.as_deref(), slot);
            assert!(!a.host.devtools, "a flag inside the URL is data");
        }
    }

    #[test]
    fn a_command_targets_its_exe_case_insensitively() {
        let c = command_line(r"C:\Apps\Notes.exe", None);
        assert!(command_targets(&c, r"c:\apps\notes.exe"));
        assert!(!command_targets(&c, r"C:\Apps\Other.exe"));
        assert!(!command_targets("short", r"C:\Apps\Notes.exe"));
    }
}
