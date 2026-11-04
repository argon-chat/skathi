//! The ONE argv parser (docs/LAUNCH.md §2). Every consumer of this process's
//! command line — the inspector mode, `--vn-host-info`, the protocol handler,
//! `appLaunch.argv` — reads THIS, never `std::env::args()` directly.
//!
//! ## Why it exists before any protocol handler does
//!
//! Until m25 `inspector::mode()` treated `--devtools` ANYWHERE in argv as "this
//! process is an inspector". The day a custom protocol is registered, a web
//! page can make the shell start this exe with a URL it chose, and a URL like
//! `myapp://x" --devtools "y` substituted into a `"%1"` command line would have
//! flipped the process mode — the Electron CVE-2018-1000006 class
//! (research/reports/m25/desktop-apis-design.md finding 6). So the grammar is
//! now fixed and small:
//!
//! ```text
//!   <exe> [host flags…] [--] [app args…]
//!   <exe> [--vn-dist <slot>] --vn-protocol-launch -- <url, verbatim>
//! ```
//!
//! * **Host flags are a LEADING RUN.** They are recognised only from argv[1]
//!   onwards up to the first argument that is not one of them (or an explicit
//!   `--`). `Hello.exe report.txt --devtools` is an app launched with two app
//!   arguments, not an inspector. That is stricter than the design's "before
//!   the first `--`" and was chosen deliberately: nothing that legitimately
//!   passes a host flag (F12's spawn, `devtools/ui.ts`, `vn build`'s host
//!   check) passes it anywhere but first.
//! * **Protocol mode honours nothing but `--vn-dist`.** The registry writes
//!   `"<exe>" [--vn-dist "<slot>"] --vn-protocol-launch -- "%1"`; any other host
//!   flag that appears in front of `--vn-protocol-launch` is ignored and
//!   reported in [`Args::ignored`]. Everything after the marker is the URL.
//! * **The URL is recovered from the RAW command line** (`GetCommandLineW`),
//!   after the `--` that follows the marker, with one surrounding pair of
//!   quotes stripped. The shell pastes the URL between the quotes verbatim — it
//!   escapes nothing — so re-splitting it with the C runtime's rules would turn
//!   a `"` or a run of spaces inside the URL into argument boundaries. Taken
//!   from the raw text it arrives byte-exact.
//!
//! Pure: no I/O and no Win32 beyond the one `GetCommandLineW` read in
//! [`process`], so it is unit-tested without a host.

use std::sync::OnceLock;

/// The marker the protocol registration writes in front of the URL.
pub const PROTOCOL_LAUNCH: &str = "--vn-protocol-launch";

/// The host flags, as this process was given them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HostFlags {
    /// `--devtools`: this process IS the embedded inspector.
    pub devtools: bool,
    /// `--target <slot>` (inspector): the slot of the app under inspection.
    pub target: Option<String>,
    /// `--window <id>` (inspector): the gpui window F12 was pressed in.
    pub window: Option<String>,
    /// `--parent <pid>` (inspector): the pid to die with.
    pub parent: Option<String>,
    /// `--vn-dist <slot>`: the dev dist slot. What a protocol registration made
    /// by a DEV host writes, because a browser-started process has no `VN_DIST`.
    pub vn_dist: Option<String>,
    /// `--vn-unregister-protocols`: remove this app's protocol keys and exit.
    pub unregister_protocols: bool,
    /// `--vn-host-info`: `vn build`'s host check (bare hosts only).
    pub host_info: bool,
}

/// This process's command line, parsed once.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    /// argv[0], as given.
    pub exe: String,
    pub host: HostFlags,
    /// Started by a protocol registration (`--vn-protocol-launch`).
    pub protocol_launch: bool,
    /// The deep link of a protocol launch, byte-exact. `None` otherwise.
    pub url: Option<String>,
    /// Everything that is DATA: after the host-flag run (and after an explicit
    /// `--`). Empty in protocol mode — the URL is not an app argument.
    pub app_args: Vec<String>,
    /// `appLaunch.argv`: the full argv as given, exe first (Electron's
    /// `process.argv` shape), minus the `--vn-*` internals and their values.
    /// In protocol mode `[exe, url]`.
    pub argv: Vec<String>,
    /// Host flags that were present but NOT honoured because this is a protocol
    /// launch. Non-empty means somebody hand-built a protocol command line —
    /// the registry never writes one.
    pub ignored: Vec<String>,
}

/// Flags that take a value. The value is the next argument unless that one
/// itself starts with `--` (the rule `inspector::mode()` always applied).
const VALUE_FLAGS: &[&str] = &["--target", "--window", "--parent", "--vn-dist"];
/// Flags that stand alone.
const BARE_FLAGS: &[&str] = &["--devtools", "--vn-unregister-protocols", "--vn-host-info"];

fn is_host_flag(a: &str) -> bool {
    VALUE_FLAGS.contains(&a) || BARE_FLAGS.contains(&a) || a == PROTOCOL_LAUNCH
}

/// Parses `argv` (argv[0] included). `raw` is the unsplit command line
/// (`GetCommandLineW`), used ONLY to recover a protocol launch's URL; without
/// it the URL is the remaining arguments joined by single spaces, which is
/// lossy for a URL that contained a `"` or a run of spaces.
pub fn parse(argv: &[String], raw: Option<&str>) -> Args {
    let mut out = Args { exe: argv.first().cloned().unwrap_or_default(), ..Args::default() };
    // `(flag, value)` in the order seen, so protocol mode can say exactly what
    // it ignored.
    let mut seen: Vec<(String, Option<String>)> = Vec::new();
    let mut i = 1;
    let mut protocol_at: Option<usize> = None;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "--" {
            i += 1;
            break;
        }
        if a == PROTOCOL_LAUNCH {
            protocol_at = Some(i);
            break;
        }
        if !is_host_flag(a) {
            break;
        }
        if VALUE_FLAGS.contains(&a) {
            let value = argv.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
            if value.is_some() {
                i += 1;
            }
            seen.push((a.to_string(), value));
        } else {
            seen.push((a.to_string(), None));
        }
        i += 1;
    }

    if let Some(p) = protocol_at {
        out.protocol_launch = true;
        for (flag, value) in seen {
            if flag == "--vn-dist" {
                out.host.vn_dist = value;
            } else {
                out.ignored.push(flag);
            }
        }
        out.url = raw
            .and_then(|r| url_from_raw(r, p))
            .or_else(|| url_from_argv(argv, p))
            .filter(|u| !u.is_empty());
        out.argv = std::iter::once(out.exe.clone()).chain(out.url.clone()).collect();
        return out;
    }

    for (flag, value) in seen {
        match flag.as_str() {
            "--devtools" => out.host.devtools = true,
            "--target" => out.host.target = value,
            "--window" => out.host.window = value,
            "--parent" => out.host.parent = value,
            "--vn-dist" => out.host.vn_dist = value,
            "--vn-unregister-protocols" => out.host.unregister_protocols = true,
            "--vn-host-info" => out.host.host_info = true,
            _ => {}
        }
    }
    out.app_args = argv.get(i..).map(<[String]>::to_vec).unwrap_or_default();

    // `argv` keeps everything the user typed except our own `--vn-*` plumbing:
    // a dev slot path is not an argument the app was launched with.
    let mut j = 1;
    out.argv.push(out.exe.clone());
    while j < argv.len() {
        let a = argv[j].as_str();
        if j < i && a.starts_with("--vn-") {
            if VALUE_FLAGS.contains(&a) && argv.get(j + 1).is_some_and(|v| !v.starts_with("--")) {
                j += 1;
            }
            j += 1;
            continue;
        }
        out.argv.push(argv[j].clone());
        j += 1;
    }
    out
}

/// The protocol URL without the raw command line: everything after the marker
/// (and its `--`), re-joined. Lossy by construction; only a fallback.
fn url_from_argv(argv: &[String], marker: usize) -> Option<String> {
    let mut k = marker + 1;
    if argv.get(k).map(String::as_str) == Some("--") {
        k += 1;
    }
    let rest = argv.get(k..)?;
    if rest.is_empty() {
        return None;
    }
    Some(rest.join(" "))
}

/// The protocol URL out of the raw command line, byte-exact.
///
/// `marker` is the index of `--vn-protocol-launch` in the C-runtime split; the
/// raw text is tokenised with the SAME rules ([`split_command_line`]) so the
/// two indices name the same token. If they disagree (they cannot for a
/// command line the registry wrote) the caller falls back to argv.
fn url_from_raw(raw: &str, marker: usize) -> Option<String> {
    let tokens = split_command_line(raw);
    if tokens.get(marker).map(|t| t.text.as_str()) != Some(PROTOCOL_LAUNCH) {
        return None;
    }
    let mut k = marker + 1;
    if tokens.get(k).map(|t| t.text.as_str()) == Some("--") {
        k += 1;
    }
    let start = tokens.get(k)?.start;
    let rest = raw[start..].trim_end_matches([' ', '\t', '\r', '\n']);
    Some(strip_one_quote_pair(rest).to_string())
}

/// `"x"` → `x`; anything else unchanged. ONE pair, so a URL that itself starts
/// and ends with a quote inside the registry's own pair keeps its quotes.
fn strip_one_quote_pair(s: &str) -> &str {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// One argument of a raw command line, and the byte offset it started at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub start: usize,
}

/// Splits a Windows command line exactly as the C runtime (and Rust's
/// `std::env::args`) does, keeping each argument's starting byte offset.
///
/// * argv[0] is special: if it starts with `"` it runs to the next `"` (no
///   escapes at all — a path cannot contain a quote); otherwise to the first
///   space or tab.
/// * after that, space/tab separate arguments outside quotes; a `"` toggles
///   quoting unless escaped; `2n` backslashes before a `"` become `n` and the
///   quote toggles, `2n+1` become `n` and a literal `"`; backslashes not
///   followed by a quote are literal; inside quotes `""` is a literal `"`.
pub fn split_command_line(raw: &str) -> Vec<Token> {
    let b = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let is_ws = |c: u8| c == b' ' || c == b'\t';

    // argv[0]
    let start = i;
    let mut text = Vec::new();
    if b.first() == Some(&b'"') {
        i += 1;
        while i < b.len() && b[i] != b'"' {
            text.push(b[i]);
            i += 1;
        }
        if i < b.len() {
            i += 1; // the closing quote
        }
        // Anything glued to the closing quote is still argv[0] (`"a"b` is `ab`).
        while i < b.len() && !is_ws(b[i]) {
            text.push(b[i]);
            i += 1;
        }
    } else {
        while i < b.len() && !is_ws(b[i]) {
            text.push(b[i]);
            i += 1;
        }
    }
    if !raw.is_empty() {
        out.push(Token { text: String::from_utf8_lossy(&text).into_owned(), start });
    }

    loop {
        while i < b.len() && is_ws(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        let mut text: Vec<u8> = Vec::new();
        let mut in_quotes = false;
        while i < b.len() {
            let c = b[i];
            if c == b'\\' {
                let mut n = 0;
                while i < b.len() && b[i] == b'\\' {
                    n += 1;
                    i += 1;
                }
                if i < b.len() && b[i] == b'"' {
                    text.extend(std::iter::repeat_n(b'\\', n / 2));
                    if n % 2 == 1 {
                        text.push(b'"');
                        i += 1;
                    }
                    // even: the quote is processed by the next iteration
                } else {
                    text.extend(std::iter::repeat_n(b'\\', n));
                }
                continue;
            }
            if c == b'"' {
                if in_quotes && b.get(i + 1) == Some(&b'"') {
                    text.push(b'"');
                    i += 2;
                    continue;
                }
                in_quotes = !in_quotes;
                i += 1;
                continue;
            }
            if is_ws(c) && !in_quotes {
                break;
            }
            text.push(c);
            i += 1;
        }
        out.push(Token { text: String::from_utf8_lossy(&text).into_owned(), start });
    }
    out
}

/// Quotes one argument so [`split_command_line`] (and the C runtime) reads it
/// back unchanged: wrapped in quotes when it is empty or contains a space, a
/// tab or a quote; backslashes doubled where they precede a quote.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    quote_always(arg)
}

/// [`quote_arg`], but ALWAYS wrapped in quotes — what a registry command line
/// wants for a path, whether or not this one happens to contain a space.
pub fn quote_always(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    // A trailing run of backslashes would escape the closing quote.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

static ARGS: OnceLock<Args> = OnceLock::new();

/// The arguments of an EMBEDDED vue-native (vn-app's src/embed.rs): the
/// process's command line belongs to the embedder, so none of it is a host
/// flag, and `appLaunch.argv` is what the embedder hands the app, `exe` first.
/// Before the first [`process`]; `false` when that already ran.
pub fn set_embedded(exe: String, app_args: Vec<String>) -> bool {
    let mut argv = vec![exe.clone()];
    argv.extend(app_args.iter().cloned());
    ARGS.set(Args { exe, app_args, argv, ..Args::default() }).is_ok()
}

/// This process's command line, parsed on first ask and never again.
pub fn process() -> &'static Args {
    ARGS.get_or_init(|| {
        let argv: Vec<String> =
            std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect();
        parse(&argv, raw_command_line().as_deref())
    })
}

/// `GetCommandLineW`, as UTF-8 (lossy for unpaired surrogates).
#[cfg(windows)]
pub fn raw_command_line() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCommandLineW() -> *const u16;
    }
    // SAFETY: the returned pointer is owned by the process and valid for its
    // lifetime; it is NUL-terminated.
    unsafe {
        let p = GetCommandLineW();
        if p.is_null() {
            return None;
        }
        let mut n = 0usize;
        while *p.add(n) != 0 {
            n += 1;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)))
    }
}

#[cfg(not(windows))]
pub fn raw_command_line() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// The C runtime's split of `raw`, so a test states ONE command line and
    /// gets both inputs `parse` sees in a real process.
    fn both(raw: &str) -> Args {
        let argv: Vec<String> = split_command_line(raw).into_iter().map(|t| t.text).collect();
        parse(&argv, Some(raw))
    }

    #[test]
    fn a_bare_launch_has_no_flags_and_no_url() {
        let a = parse(&v(&["app.exe"]), None);
        assert_eq!(a.host, HostFlags::default());
        assert!(!a.protocol_launch);
        assert_eq!(a.url, None);
        assert!(a.app_args.is_empty());
        assert_eq!(a.argv, v(&["app.exe"]));
    }

    #[test]
    fn the_inspector_command_line_still_parses() {
        // Exactly what `inspector::spawn` builds.
        let a = parse(&v(&["vn-app.exe", "--devtools", "--target", "C:\\slot", "--parent", "42", "--window", "7"]), None);
        assert!(a.host.devtools);
        assert_eq!(a.host.target.as_deref(), Some("C:\\slot"));
        assert_eq!(a.host.parent.as_deref(), Some("42"));
        assert_eq!(a.host.window.as_deref(), Some("7"));
        assert!(a.app_args.is_empty());
    }

    #[test]
    fn devtools_after_an_app_argument_is_data() {
        // The old `any(|a| a == "--devtools")` would have flipped the mode here.
        let a = parse(&v(&["app.exe", "report.txt", "--devtools"]), None);
        assert!(!a.host.devtools);
        assert_eq!(a.app_args, v(&["report.txt", "--devtools"]));
        assert_eq!(a.argv, v(&["app.exe", "report.txt", "--devtools"]));
    }

    #[test]
    fn devtools_after_a_double_dash_is_data() {
        let a = parse(&v(&["app.exe", "--", "--devtools", "--target", "x"]), None);
        assert!(!a.host.devtools);
        assert_eq!(a.host.target, None);
        assert_eq!(a.app_args, v(&["--devtools", "--target", "x"]));
    }

    #[test]
    fn an_unknown_leading_flag_ends_the_host_run() {
        let a = parse(&v(&["app.exe", "--verbose", "--devtools"]), None);
        assert!(!a.host.devtools);
        assert_eq!(a.app_args, v(&["--verbose", "--devtools"]));
    }

    #[test]
    fn a_value_flag_does_not_swallow_the_next_flag() {
        let a = parse(&v(&["app.exe", "--devtools", "--target", "--parent", "9"]), None);
        assert!(a.host.devtools);
        assert_eq!(a.host.target, None);
        assert_eq!(a.host.parent.as_deref(), Some("9"));
    }

    #[test]
    fn vn_internals_are_not_part_of_app_argv() {
        let a = parse(&v(&["app.exe", "--vn-dist", "C:\\s", "notes.md"]), None);
        assert_eq!(a.host.vn_dist.as_deref(), Some("C:\\s"));
        assert_eq!(a.app_args, v(&["notes.md"]));
        assert_eq!(a.argv, v(&["app.exe", "notes.md"]));
        // …but a `--vn-` string an app was handed as DATA stays.
        let b = parse(&v(&["app.exe", "--", "--vn-dist", "x"]), None);
        assert_eq!(b.argv, v(&["app.exe", "--", "--vn-dist", "x"]));
        assert_eq!(b.host.vn_dist, None);
    }

    #[test]
    fn host_info_and_unregister_are_leading_flags() {
        assert!(parse(&v(&["h.exe", "--vn-host-info"]), None).host.host_info);
        assert!(!parse(&v(&["h.exe", "x", "--vn-host-info"]), None).host.host_info);
        assert!(parse(&v(&["h.exe", "--vn-unregister-protocols"]), None).host.unregister_protocols);
    }

    #[test]
    fn a_protocol_launch_recovers_the_url_byte_exact() {
        // The shell pastes the URL into `"%1"` verbatim, quotes and all.
        let a = both(r#""C:\Program Files\Hello\Hello.exe" --vn-protocol-launch -- "myapp://a?b="c d"""#);
        assert!(a.protocol_launch);
        assert_eq!(a.url.as_deref(), Some(r#"myapp://a?b="c d""#));
        assert!(a.app_args.is_empty());
        assert_eq!(a.argv, v(&["C:\\Program Files\\Hello\\Hello.exe", r#"myapp://a?b="c d""#]));
    }

    #[test]
    fn a_protocol_url_keeps_spaces_and_percent_escapes() {
        let a = both(r#""C:\h.exe" --vn-protocol-launch -- "myapp://x/a  b%22c%20d""#);
        assert_eq!(a.url.as_deref(), Some("myapp://x/a  b%22c%20d"));
    }

    #[test]
    fn an_injected_devtools_inside_the_url_flips_nothing() {
        // The CVE-2018-1000006 shape: a URL that closes the registry's quote.
        let url = r#"myapp://x" --devtools --target "C:\evil"#;
        let a = both(&format!(r#""C:\h.exe" --vn-protocol-launch -- "{url}""#));
        assert!(!a.host.devtools);
        assert_eq!(a.host.target, None);
        assert_eq!(a.url.as_deref(), Some(url));
        assert!(a.ignored.is_empty());
    }

    #[test]
    fn host_flags_in_front_of_the_marker_are_ignored_except_vn_dist() {
        let a = both(r#""C:\h.exe" --devtools --vn-dist "D:\dev slot" --target x --vn-protocol-launch -- "myapp://y""#);
        assert!(a.protocol_launch);
        assert!(!a.host.devtools);
        assert_eq!(a.host.target, None);
        assert_eq!(a.host.vn_dist.as_deref(), Some("D:\\dev slot"));
        assert_eq!(a.ignored, v(&["--devtools", "--target"]));
        assert_eq!(a.url.as_deref(), Some("myapp://y"));
    }

    #[test]
    fn without_the_raw_line_the_url_is_rejoined() {
        let a = parse(&v(&["h.exe", "--vn-protocol-launch", "--", "myapp://a", "b"]), None);
        assert_eq!(a.url.as_deref(), Some("myapp://a b"));
    }

    #[test]
    fn a_protocol_launch_with_no_url_has_none() {
        let a = both(r#""C:\h.exe" --vn-protocol-launch --"#);
        assert!(a.protocol_launch);
        assert_eq!(a.url, None);
        assert_eq!(a.argv, v(&["C:\\h.exe"]));
    }

    #[test]
    fn the_tokenizer_follows_the_c_runtime_rules() {
        let t = |raw: &str| -> Vec<String> { split_command_line(raw).into_iter().map(|t| t.text).collect() };
        assert_eq!(t(r#""C:\a b\x.exe" one "two three""#), v(&["C:\\a b\\x.exe", "one", "two three"]));
        // argv[0] takes no escapes.
        assert_eq!(t(r#"C:\x\y.exe a"#), v(&["C:\\x\\y.exe", "a"]));
        // 2n backslashes + quote → n backslashes, quote toggles.
        assert_eq!(t(r#"x "a\\" b"#), v(&["x", "a\\", "b"]));
        // 2n+1 backslashes + quote → n backslashes + a literal quote.
        assert_eq!(t(r#"x a\"b"#), v(&["x", "a\"b"]));
        // backslashes not before a quote are literal.
        assert_eq!(t(r#"x a\\b"#), v(&["x", "a\\\\b"]));
        // `""` inside quotes is a literal quote.
        assert_eq!(t(r#"x "a""b""#), v(&["x", "a\"b"]));
        // runs of whitespace are one separator; offsets point at the token.
        let toks = split_command_line("x   a\tb");
        assert_eq!(toks.iter().map(|t| t.start).collect::<Vec<_>>(), vec![0, 4, 6]);
    }

    #[test]
    fn quote_arg_round_trips_through_the_tokenizer() {
        for arg in ["plain", "with space", "C:\\dir with space\\", "a\"b", "", "trail\\", "x\\\"y", "\t"] {
            let raw = format!("exe {}", quote_arg(arg));
            let toks = split_command_line(&raw);
            assert_eq!(toks.len(), 2, "{raw}");
            assert_eq!(toks[1].text, arg, "{raw}");
        }
    }
}
