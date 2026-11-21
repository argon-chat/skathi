//! `skathi:encoding`: bytes as text and back (hex, Base32, Base64, legacy encodings) and MIME types.

use std::ffi::c_void;

use encoding_rs::{EncoderResult, Encoding, UTF_16BE, UTF_16LE};
use sk_runtime::redjs::webapi::mime;
use sk_runtime::redjs::webapi::util::*;

use super::support::*;

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    let digits = text.as_bytes();
    if digits.len() % 2 != 0 {
        return Err(format!(
            "{} hex digits is not a whole number of bytes",
            digits.len()
        ));
    }
    let digit = |at: usize| {
        (digits[at] as char)
            .to_digit(16)
            .ok_or_else(|| format!("character {at} is not a hex digit"))
    };
    (0..digits.len() / 2)
        .map(|i| Ok((digit(i * 2)? << 4 | digit(i * 2 + 1)?) as u8))
        .collect()
}

fn hex_dump(bytes: &[u8], width: usize, offset: usize) -> String {
    let mut out = String::new();
    for (row, line) in bytes.chunks(width).enumerate() {
        out.push_str(&format!("{:08x}:", offset + row * width));
        for i in 0..width {
            if i % 2 == 0 {
                out.push(' ');
            }
            match line.get(i) {
                Some(byte) => out.push_str(&format!("{byte:02x}")),
                None => out.push_str("  "),
            }
        }
        out.push_str("  ");
        out.extend(line.iter().map(|b| {
            if b.is_ascii_graphic() || *b == b' ' {
                *b as char
            } else {
                '.'
            }
        }));
        out.push('\n');
    }
    out
}

const BASE32: [&[u8; 32]; 3] = [
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567",
    b"0123456789ABCDEFGHIJKLMNOPQRSTUV",
    b"0123456789ABCDEFGHJKMNPQRSTVWXYZ",
];
const BASE32_NAMES: &[&str] = &["rfc4648", "hex", "crockford"];
const CROCKFORD: usize = 2;

fn base32_encode(bytes: &[u8], alphabet: usize, padding: bool) -> String {
    let table = BASE32[alphabet];
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut acc, mut bits) = (0u32, 0);
    for byte in bytes {
        acc = (acc << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(table[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(table[((acc << (5 - bits)) & 31) as usize] as char);
    }
    while padding && out.len() % 8 != 0 {
        out.push('=');
    }
    out
}

fn base32_decode(text: &str, alphabet: usize) -> Result<Vec<u8>, String> {
    let table = BASE32[alphabet];
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut acc, mut bits) = (0u32, 0);
    for (at, c) in text.trim_end_matches('=').char_indices() {
        if c.is_ascii_whitespace() || (alphabet == CROCKFORD && c == '-') {
            continue;
        }
        let mut upper = c.to_ascii_uppercase();
        if alphabet == CROCKFORD {
            upper = match upper {
                'O' => '0',
                'I' | 'L' => '1',
                other => other,
            };
        }
        let Some(value) = table.iter().position(|t| *t as char == upper) else {
            return Err(format!("character {at} is not in the alphabet"));
        };
        acc = (acc << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

const BASE64: [&[u8; 64]; 2] = [
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_",
];
const BASE64_NAMES: &[&str] = &["base64", "base64url"];

fn base64_encode(bytes: &[u8], alphabet: usize, padding: bool) -> String {
    let table = BASE64[alphabet];
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..=chunk.len() {
            out.push(table[((word >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    while padding && out.len() % 4 != 0 {
        out.push('=');
    }
    out
}

/// `alphabet` of `None` takes the characters of both.
fn base64_decode(text: &str, alphabet: Option<usize>) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits, mut symbols) = (0u32, 0, 0usize);
    for (at, c) in text
        .trim_end_matches(|c: char| c == '=' || c.is_ascii_whitespace())
        .char_indices()
    {
        if c.is_ascii_whitespace() {
            continue;
        }
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' if alphabet != Some(1) => 62,
            '/' if alphabet != Some(1) => 63,
            '-' if alphabet != Some(0) => 62,
            '_' if alphabet != Some(0) => 63,
            _ => return Err(format!("character {at} is not in the alphabet")),
        };
        symbols += 1;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if symbols % 4 == 1 {
        return Err("the last group has one character, which is less than a byte".into());
    }
    Ok(out)
}

fn starts(bytes: &[u8], at: usize, magic: &[u8]) -> bool {
    bytes.len() >= at + magic.len() && &bytes[at..at + magic.len()] == magic
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    const MAGIC: &[(usize, &[u8], &str)] = &[
        (0, b"\x89PNG\r\n\x1a\n", "image/png"),
        (0, b"\xff\xd8\xff", "image/jpeg"),
        (0, b"GIF87a", "image/gif"),
        (0, b"GIF89a", "image/gif"),
        (0, b"BM", "image/bmp"),
        (0, b"\x00\x00\x01\x00", "image/x-icon"),
        (0, b"\x00\x00\x02\x00", "image/x-icon"),
        (0, b"%PDF-", "application/pdf"),
        (0, b"%!PS-Adobe-", "application/postscript"),
        (0, b"PK\x03\x04", "application/zip"),
        (0, b"\x1f\x8b\x08", "application/gzip"),
        (0, b"Rar!\x1a\x07", "application/vnd.rar"),
        (0, b"7z\xbc\xaf\x27\x1c", "application/x-7z-compressed"),
        (0, b"\xfd7zXZ\x00", "application/x-xz"),
        (0, b"\x28\xb5\x2f\xfd", "application/zstd"),
        (0, b"BZh", "application/x-bzip2"),
        (0, b"\x00asm", "application/wasm"),
        (0, b"SQLite format 3\x00", "application/vnd.sqlite3"),
        (0, b"wOFF", "font/woff"),
        (0, b"wOF2", "font/woff2"),
        (0, b"\x00\x01\x00\x00", "font/ttf"),
        (0, b"OTTO", "font/otf"),
        (0, b"ttcf", "font/collection"),
        (0, b"OggS", "application/ogg"),
        (0, b"fLaC", "audio/flac"),
        (0, b"MThd", "audio/midi"),
        (0, b"ID3", "audio/mpeg"),
        (0, b"\xff\xfb", "audio/mpeg"),
        (0, b"\xff\xf3", "audio/mpeg"),
        (0, b"\xff\xf2", "audio/mpeg"),
    ];
    if let Some((_, _, found)) = MAGIC
        .iter()
        .find(|(at, magic, _)| starts(bytes, *at, magic))
    {
        return Some(found);
    }
    if starts(bytes, 0, b"RIFF") {
        return [
            (&b"WEBP"[..], "image/webp"),
            (b"WAVE", "audio/wav"),
            (b"AVI ", "video/x-msvideo"),
        ]
        .iter()
        .find(|(form, _)| starts(bytes, 8, form))
        .map(|(_, found)| *found);
    }
    if starts(bytes, 0, b"FORM") && starts(bytes, 8, b"AIFF") {
        return Some("audio/aiff");
    }
    if starts(bytes, 4, b"ftyp") && bytes.len() >= 12 {
        return Some(match &bytes[8..12] {
            b"avif" | b"avis" => "image/avif",
            b"heic" | b"heix" | b"mif1" => "image/heic",
            b"qt  " => "video/quicktime",
            b"M4A " => "audio/mp4",
            _ => "video/mp4",
        });
    }
    if starts(bytes, 0, b"\x1a\x45\xdf\xa3") {
        let head = &bytes[..bytes.len().min(64)];
        let webm = head.windows(4).any(|w| w == b"webm");
        return Some(if webm {
            "video/webm"
        } else {
            "video/x-matroska"
        });
    }
    let bom = [&b"\xef\xbb\xbf"[..], b"\xfe\xff", b"\xff\xfe"]
        .into_iter()
        .find(|bom| starts(bytes, 0, bom));
    let text = &bytes[bom.map_or(0, |b| b.len())..];
    let text = &text[text.iter().take_while(|b| b.is_ascii_whitespace()).count()..];
    let head = String::from_utf8_lossy(&text[..text.len().min(512)]).to_ascii_lowercase();
    if head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")) {
        return Some("image/svg+xml");
    }
    if head.starts_with("<?xml") {
        return Some("text/xml");
    }
    if head.starts_with("<!doctype html") || head.starts_with("<html") {
        return Some("text/html");
    }
    bom.map(|_| "text/plain")
}

/// Whether `kind` is one of the comma-separated `accept` entries: a type, `type/*`, `*/*` or `.ext`.
fn accepts(kind: &str, accept: &str) -> bool {
    let Some(kind) = mime::essence(kind) else {
        return false;
    };
    accept.split(',').any(|entry| {
        let entry = entry
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if let Some(extension) = entry.strip_prefix('.') {
            return mime::exts_for(&kind).iter().any(|e| e == extension);
        }
        match entry.split_once('/') {
            Some(("*", "*")) => true,
            Some((top, "*")) => kind.split_once('/').is_some_and(|(t, _)| t == top),
            Some(_) => entry == kind,
            None => false,
        }
    })
}

fn detect(bytes: &[u8]) -> Option<&'static str> {
    Encoding::for_bom(bytes).map(|(encoding, _)| {
        if encoding == UTF_16LE {
            "utf-16le"
        } else if encoding == UTF_16BE {
            "utf-16be"
        } else {
            "utf-8"
        }
    })
}

/// `text` in `encoding`; a character it has no bytes for becomes `?`.
fn encode(text: &str, encoding: &'static Encoding) -> Vec<u8> {
    if encoding == UTF_16LE {
        return text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    }
    if encoding == UTF_16BE {
        return text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    }
    let mut encoder = encoding.new_encoder();
    let mut out = Vec::with_capacity(text.len() + 16);
    let mut rest = text;
    loop {
        let room = encoder
            .max_buffer_length_from_utf8_without_replacement(rest.len())
            .unwrap_or(rest.len() * 4 + 16);
        out.reserve(room);
        let (result, read) =
            encoder.encode_from_utf8_to_vec_without_replacement(rest, &mut out, true);
        rest = &rest[read..];
        match result {
            EncoderResult::InputEmpty => return out,
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(_) => out.push(b'?'),
        }
    }
}

unsafe fn encoding_for(g: *mut c_void, v: u64, what: &str) -> Js<&'static Encoding> {
    let label = string(g, v, &format!("{what}: the label"))?;
    match Encoding::for_label(label.as_bytes()) {
        Some(encoding) => Ok(encoding),
        None => range_error(
            g,
            &format!("{what}: \"{label}\" is not an encoding this host knows"),
        ),
    }
}

std_fn!(fn hex_encode_fn(g, args) {
    Ok(js_string(g, &hex_encode(bytes_arg(g, args.arg(0).0, "Hex.encode: the bytes")?)))
});

std_fn!(fn hex_decode_fn(g, args) {
    let text = string(g, args.arg(0).0, "Hex.decode: the text")?;
    match hex_decode(&text) {
        Ok(bytes) => Ok(make_u8(g, &bytes)),
        Err(reason) => syntax_error(g, &format!("Hex.decode: {reason}")),
    }
});

std_fn!(fn hex_dump_fn(g, args) {
    let options = args.arg(1).0;
    let width = match option(g, options, "width") {
        UNDEF => 16,
        v => count(g, v, "Hex.dump: width")?,
    };
    if !(1..=256).contains(&width) {
        return range_error(g, "Hex.dump: width must be from 1 to 256");
    }
    let offset = match option(g, options, "offset") {
        UNDEF => 0,
        v => count(g, v, "Hex.dump: offset")?,
    };
    let bytes = bytes_arg(g, args.arg(0).0, "Hex.dump: the bytes")?;
    Ok(js_string(g, &hex_dump(bytes, width, offset)))
});

std_fn!(fn hex_format(g, args) {
    let value = args.arg(0).0;
    let width = match args.arg(1).0 {
        UNDEF => 0,
        v => count(g, v, "Hex.format: the width")?.min(64),
    };
    let (negative, digits) = if kind(g, value) == KIND_BIGINT {
        let text = call(g, get(g, value, "toString"), value, &[enc_i32(16)]).map_err(|_| Thrown)?;
        let text = read_string(g, text).unwrap_or_default();
        match text.strip_prefix('-') {
            Some(digits) => (true, digits.to_owned()),
            None => (false, text),
        }
    } else {
        let n = number(g, value)?;
        if n.fract() != 0.0 || n.abs() > 9_007_199_254_740_992.0 {
            return range_error(g, "Hex.format: the value must be an integer or a bigint");
        }
        (n < 0.0, format!("{:x}", n.abs() as u64))
    };
    let sign = if negative { "-" } else { "" };
    Ok(js_string(g, &format!("{sign}0x{digits:0>width$}")))
});

std_fn!(fn base32_encode_fn(g, args) {
    let options = args.arg(1).0;
    let alphabet = choice(g, option(g, options, "alphabet"), BASE32_NAMES, 0, "Base32.encode: alphabet")?;
    let padding = match option(g, options, "padding") {
        UNDEF => alphabet != CROCKFORD,
        v => to_bool(g, v),
    };
    let bytes = bytes_arg(g, args.arg(0).0, "Base32.encode: the bytes")?;
    Ok(js_string(g, &base32_encode(bytes, alphabet, padding)))
});

std_fn!(fn base32_decode_fn(g, args) {
    let text = string(g, args.arg(0).0, "Base32.decode: the text")?;
    let alphabet = choice(g, option(g, args.arg(1).0, "alphabet"), BASE32_NAMES, 0, "Base32.decode: alphabet")?;
    match base32_decode(&text, alphabet) {
        Ok(bytes) => Ok(make_u8(g, &bytes)),
        Err(reason) => syntax_error(g, &format!("Base32.decode: {reason}")),
    }
});

std_fn!(fn base64_encode_fn(g, args) {
    let options = args.arg(1).0;
    let alphabet = choice(g, option(g, options, "alphabet"), BASE64_NAMES, 0, "Base64.encode: alphabet")?;
    let padding = !to_bool(g, option(g, options, "omitPadding"));
    let bytes = bytes_arg(g, args.arg(0).0, "Base64.encode: the bytes")?;
    Ok(js_string(g, &base64_encode(bytes, alphabet, padding)))
});

std_fn!(fn base64_decode_fn(g, args) {
    let text = string(g, args.arg(0).0, "Base64.decode: the text")?;
    let alphabet = match option(g, args.arg(1).0, "alphabet") {
        UNDEF => None,
        v => Some(choice(g, v, BASE64_NAMES, 0, "Base64.decode: alphabet")?),
    };
    match base64_decode(&text, alphabet) {
        Ok(bytes) => Ok(make_u8(g, &bytes)),
        Err(reason) => syntax_error(g, &format!("Base64.decode: {reason}")),
    }
});

std_fn!(fn mime_from_name(g, args) {
    let name = string(g, args.arg(0).0, "Mime.fromName: the name")?;
    Ok(js_string(g, mime::type_for_name(&name).unwrap_or("")))
});

std_fn!(fn mime_extensions(g, args) {
    let kind = string(g, args.arg(0).0, "Mime.extensions: the type")?;
    let list = new_array(g);
    for (i, extension) in mime::exts_for(&kind).iter().enumerate() {
        set_i(g, list, i as u32, js_string(g, extension));
    }
    Ok(list)
});

std_fn!(fn mime_sniff(g, args) {
    let bytes = bytes_arg(g, args.arg(0).0, "Mime.sniff: the bytes")?;
    Ok(match sniff(bytes) {
        Some(found) => js_string(g, found),
        None => NULL,
    })
});

std_fn!(fn mime_matches(g, args) {
    let kind = string(g, args.arg(0).0, "Mime.matches: the type")?;
    let accept = string(g, args.arg(1).0, "Mime.matches: the accept list")?;
    Ok(enc_bool(accepts(&kind, &accept)))
});

std_fn!(fn encoding_decode(g, args) {
    let encoding = encoding_for(g, args.arg(1).0, "Encoding.decode")?;
    let bytes = bytes_arg(g, args.arg(0).0, "Encoding.decode: the bytes")?;
    let (text, _) = encoding.decode_with_bom_removal(bytes);
    Ok(js_string(g, &text))
});

std_fn!(fn encoding_encode(g, args) {
    let text = string(g, args.arg(0).0, "Encoding.encode: the text")?;
    let encoding = encoding_for(g, args.arg(1).0, "Encoding.encode")?;
    if encoding == encoding_rs::REPLACEMENT {
        return range_error(g, "Encoding.encode: this label names an encoding that can only be decoded");
    }
    Ok(make_u8(g, &encode(&text, encoding)))
});

std_fn!(fn encoding_is_supported(g, args) {
    let supported = kind(g, args.arg(0).0) == KIND_STRING
        && read_string(g, args.arg(0).0).is_some_and(|l| Encoding::for_label(l.as_bytes()).is_some());
    Ok(enc_bool(supported))
});

std_fn!(fn encoding_detect(g, args) {
    let bytes = bytes_arg(g, args.arg(0).0, "Encoding.detect: the bytes")?;
    Ok(match detect(bytes) {
        Some(label) => js_string(g, label),
        None => NULL,
    })
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let module = new_object(g, UNDEF);
    let hex: &[(&str, u32, HostFn)] = &[
        ("encode", 1, hex_encode_fn),
        ("decode", 1, hex_decode_fn),
        ("dump", 1, hex_dump_fn),
        ("format", 1, hex_format),
    ];
    define(g, module, "Base16", namespace(g, "Base16", &hex[..2]), 0);
    define(g, module, "Hex", namespace(g, "Hex", hex), 0);
    define(
        g,
        module,
        "Base32",
        namespace(
            g,
            "Base32",
            &[
                ("encode", 1, base32_encode_fn),
                ("decode", 1, base32_decode_fn),
            ],
        ),
        0,
    );
    define(
        g,
        module,
        "Base64",
        namespace(
            g,
            "Base64",
            &[
                ("encode", 1, base64_encode_fn),
                ("decode", 1, base64_decode_fn),
            ],
        ),
        0,
    );
    define(
        g,
        module,
        "Mime",
        namespace(
            g,
            "Mime",
            &[
                ("fromName", 1, mime_from_name),
                ("extensions", 1, mime_extensions),
                ("sniff", 1, mime_sniff),
                ("matches", 2, mime_matches),
            ],
        ),
        0,
    );
    define(
        g,
        module,
        "Encoding",
        namespace(
            g,
            "Encoding",
            &[
                ("decode", 2, encoding_decode),
                ("encode", 2, encoding_encode),
                ("isSupported", 1, encoding_is_supported),
                ("detect", 1, encoding_detect),
            ],
        ),
        0,
    );
    define(g, std, "encoding", module, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_the_rfc_vectors_and_reads_crockford_by_hand() {
        assert_eq!(base32_encode(b"foobar", 0, true), "MZXW6YTBOI======");
        assert_eq!(base32_encode(b"fooba", 1, true), "CPNMUOJ1");
        assert_eq!(base32_encode(b"fo", 0, false), "MZXQ");
        assert_eq!(base32_decode("mzxw6ytboi======", 0).unwrap(), b"foobar");
        let crockford = base32_encode(b"\xff\x00\x10", CROCKFORD, false);
        assert_eq!(
            base32_decode(&crockford.replace('0', "o"), CROCKFORD).unwrap(),
            b"\xff\x00\x10"
        );
        assert!(base32_decode("M1", 0).is_err());
    }

    #[test]
    fn base64_round_trips_and_keeps_the_alphabets_apart_when_told() {
        assert_eq!(base64_encode(b"\xfb\xff", 0, true), "+/8=");
        assert_eq!(base64_encode(b"\xfb\xff", 1, false), "-_8");
        assert_eq!(base64_decode("-_8", None).unwrap(), b"\xfb\xff");
        assert_eq!(base64_decode("+/8=\n", None).unwrap(), b"\xfb\xff");
        assert!(base64_decode("-_8", Some(0)).is_err());
        assert!(base64_decode("A", None).is_err());
    }

    #[test]
    fn a_legacy_encoding_takes_a_question_mark_for_what_it_lacks() {
        let windows_1251 = Encoding::for_label(b"windows-1251").unwrap();
        assert_eq!(encode("Да€日", windows_1251), b"\xc4\xe0\x88?");
        assert_eq!(encode("é", UTF_16BE), b"\x00\xe9");
    }

    #[test]
    fn sniffing_and_accept_lists() {
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"\x00\x00\x00\x1cftypavif"), Some("image/avif"));
        assert_eq!(
            sniff(b"\xef\xbb\xbf  <?xml version='1.0'?><svg/>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"plain"), None);
        assert!(accepts("image/PNG; q=1", "text/html, image/*"));
        assert!(accepts("image/png", ".png"));
        assert!(!accepts("image/png", "image/jpeg, .gif"));
    }

    #[test]
    fn a_dump_pads_its_last_line() {
        assert_eq!(
            hex_dump(b"Hi!\x00", 3, 0x10),
            "00000010: 4869 21  Hi!\n00000013: 00       .\n"
        );
    }
}
