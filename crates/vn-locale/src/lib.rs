//! The SDK's native text in the user's language, and `appLocale` for an app's own strings.
//! Built-in strings keep `{app}` the grammatical subject with no gendered verb, so any app name fits.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard, OnceLock, RwLock};

use vn_runtime::redjs::webapi::navigator;
use vn_runtime::redjs::webapi::util::read_string;
use vn_runtime::{host_fn, JsRuntime};

include!(concat!(env!("OUT_DIR"), "/builtin.rs"));

pub mod keys {
    pub const THIS_APP: &str = "vn.app.this_app";
    pub const CONSENT_MICROPHONE_TITLE: &str = "vn.consent.microphone.title";
    pub const CONSENT_MICROPHONE_BODY: &str = "vn.consent.microphone.body";
    pub const CONSENT_CAMERA_TITLE: &str = "vn.consent.camera.title";
    pub const CONSENT_CAMERA_BODY: &str = "vn.consent.camera.body";
    pub const CONSENT_ALLOW: &str = "vn.consent.allow";
    pub const CONSENT_BLOCK: &str = "vn.consent.block";
    pub const CRASH_TITLE: &str = "vn.crash.title";
    pub const CRASH_BODY: &str = "vn.crash.body";
    pub const CRASH_BODY_NO_RESTART: &str = "vn.crash.body_no_restart";
    pub const CRASH_DETAILS: &str = "vn.crash.details";
    pub const CRASH_RESTART: &str = "vn.crash.restart";
    pub const CRASH_COPY: &str = "vn.crash.copy";
    pub const CRASH_COPIED: &str = "vn.crash.copied";
    pub const CRASH_CLOSE: &str = "vn.crash.close";
    pub const CRASH_VERSION: &str = "vn.crash.version";
}

type Catalog = HashMap<String, String>;

fn builtin() -> &'static [(String, Catalog)] {
    static PARSED: OnceLock<Vec<(String, Catalog)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        BUILTIN
            .iter()
            .map(|(tag, json)| {
                let catalog = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("locales/{tag}.json: {e}"));
                (tag.to_string(), catalog)
            })
            .collect()
    })
}

static APP: Mutex<Vec<(String, Catalog)>> = Mutex::new(Vec::new());
static LANGUAGE: RwLock<Option<String>> = RwLock::new(None);

fn app() -> MutexGuard<'static, Vec<(String, Catalog)>> {
    APP.lock().unwrap_or_else(|p| p.into_inner())
}

fn tag_of(raw: &str) -> Result<String, String> {
    navigator::canonical_tag(raw).ok_or_else(|| format!("{raw:?} is not a BCP 47 language tag"))
}

pub fn set_language(tag: Option<&str>) -> Result<(), String> {
    let tag = match tag.map(str::trim).filter(|t| !t.is_empty()) {
        None => None,
        Some(t) => Some(tag_of(t)?),
    };
    *LANGUAGE.write().unwrap_or_else(|p| p.into_inner()) = tag;
    Ok(())
}

pub fn language_override() -> Option<String> {
    LANGUAGE.read().unwrap_or_else(|p| p.into_inner()).clone()
}

pub fn preferences() -> Vec<String> {
    let mut out: Vec<String> = language_override().into_iter().collect();
    out.extend(navigator::languages());
    out
}

pub fn available() -> Vec<String> {
    let mut out: Vec<String> = builtin().iter().map(|(t, _)| t.clone()).collect();
    for (t, _) in app().iter() {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.clone());
        }
    }
    out
}

pub fn define(
    tag: &str,
    entries: impl IntoIterator<Item = (String, String)>,
) -> Result<(), String> {
    let tag = tag_of(tag)?;
    let mut app = app();
    match app.iter_mut().find(|(t, _)| t.eq_ignore_ascii_case(&tag)) {
        Some((_, catalog)) => catalog.extend(entries),
        None => app.push((tag, entries.into_iter().collect())),
    }
    Ok(())
}

pub fn language() -> String {
    negotiate(&preferences(), &available())
}

/// `zh-TW` -> `zh-Hant-TW`, `zh` -> `zh-Hans`.
fn with_script(tag: &str) -> String {
    let parts: Vec<&str> = tag.split('-').collect();
    if !parts[0].eq_ignore_ascii_case("zh") || parts.iter().skip(1).any(|p| p.len() == 4) {
        return tag.to_string();
    }
    let traditional = parts
        .get(1)
        .is_some_and(|r| ["TW", "HK", "MO"].iter().any(|t| r.eq_ignore_ascii_case(t)));
    let mut out = vec!["zh", if traditional { "Hant" } else { "Hans" }];
    out.extend(&parts[1..]);
    out.join("-")
}

fn truncations(tag: &str) -> Vec<String> {
    let parts: Vec<&str> = tag.split('-').collect();
    (1..=parts.len())
        .rev()
        .map(|n| parts[..n].join("-"))
        .collect()
}

fn base(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
}

/// Per preference: the tag or a shorter form, else another variant of its language (`pt-PT` -> `pt-BR`).
pub fn negotiate(preferences: &[String], available: &[String]) -> String {
    let find = |t: &str| {
        available
            .iter()
            .find(|a| a.eq_ignore_ascii_case(t))
            .cloned()
    };
    for p in preferences {
        let p = with_script(p);
        if let Some(a) = truncations(&p).iter().find_map(|t| find(t)) {
            return a;
        }
        if let Some(a) = available
            .iter()
            .find(|a| base(a).eq_ignore_ascii_case(base(&p)))
        {
            return a.clone();
        }
    }
    find("en")
        .or_else(|| available.first().cloned())
        .unwrap_or_else(|| "en".to_string())
}

pub fn text_in(lang: &str, key: &str) -> Option<String> {
    let app = app();
    let mut chain = truncations(&with_script(lang));
    chain.push("en".to_string());
    let lookup = |catalogs: &[(String, Catalog)], tag: &str| {
        catalogs
            .iter()
            .find(|(t, _)| t.eq_ignore_ascii_case(tag))
            .and_then(|(_, c)| c.get(key))
            .cloned()
    };
    chain
        .iter()
        .find_map(|tag| lookup(&app, tag).or_else(|| lookup(builtin(), tag)))
}

pub fn text(key: &str) -> String {
    text_in(&language(), key).unwrap_or_else(|| key.to_string())
}

pub fn format(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').map(|close| (&after[..close], close)) {
            Some((name, close)) if args.iter().any(|(n, _)| *n == name) => {
                out.push_str(args.iter().find(|(n, _)| *n == name).map_or("", |(_, v)| v));
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn t(key: &str, args: &[(&str, &str)]) -> String {
    format(&text(key), args)
}

pub fn app_name(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => text(keys::THIS_APP),
    }
}

fn js_string(s: &str) -> u64 {
    JsRuntime::with(|rt| rt.ctx().string(s).0)
}

fn state_json() -> String {
    serde_json::json!({
        "language": language(),
        "override": language_override(),
        "preferences": preferences(),
        "available": available(),
    })
    .to_string()
}

fn define_json(json: &str) -> Result<(), String> {
    let all: HashMap<String, HashMap<String, String>> = serde_json::from_str(json)
        .map_err(|e| format!("appLocale.define expects {{ tag: {{ key: text }} }}: {e}"))?;
    for tag in all.keys() {
        tag_of(tag)?;
    }
    for (tag, entries) in all {
        define(&tag, entries)?;
    }
    Ok(())
}

host_fn!(fn vn_locale_state(_g, _args) {
    js_string(&state_json())
});

host_fn!(fn vn_locale_set_language(g, args) {
    let tag = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    js_string(&set_language(Some(&tag)).err().unwrap_or_default())
});

host_fn!(fn vn_locale_define(g, args) {
    let json = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    js_string(&define_json(&json).err().unwrap_or_default())
});

host_fn!(fn vn_locale_t(g, args) {
    let key = unsafe { read_string(g, args.arg(0).0) }.unwrap_or_default();
    let raw = unsafe { read_string(g, args.arg(1).0) }.unwrap_or_default();
    let named: HashMap<String, String> = serde_json::from_str(&raw).unwrap_or_default();
    let named: Vec<(&str, &str)> = named.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    js_string(&t(&key, &named))
});

pub fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const c_void)] = &[
        ("__vn_locale_state", 0, vn_locale_state as *const _),
        (
            "__vn_locale_set_language",
            1,
            vn_locale_set_language as *const _,
        ),
        ("__vn_locale_define", 1, vn_locale_define as *const _),
        ("__vn_locale_t", 2, vn_locale_t as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn placeholders(text: &str) -> Vec<&str> {
        let mut out: Vec<&str> = text
            .split('{')
            .skip(1)
            .filter_map(|p| p.split_once('}').map(|(n, _)| n))
            .collect();
        out.sort();
        out
    }

    fn mnemonic(label: &str) -> Option<char> {
        let mut chars = label.chars();
        let mut found = None;
        while let Some(c) = chars.next() {
            if c == '&' {
                match chars.next() {
                    Some('&') | None => {}
                    Some(k) if found.is_none() => found = k.to_lowercase().next(),
                    Some(_) => return None,
                }
            }
        }
        found
    }

    #[test]
    fn every_built_in_language_has_every_key_with_the_same_placeholders() {
        let en = &builtin()
            .iter()
            .find(|(t, _)| t == "en")
            .expect("en.json")
            .1;
        assert!(builtin().len() >= 20);
        for (tag, catalog) in builtin() {
            assert_eq!(
                navigator::canonical_tag(tag).as_deref(),
                Some(tag.as_str()),
                "locales/{tag}.json: not a canonical tag"
            );
            for (key, text) in en {
                let got = catalog
                    .get(key)
                    .unwrap_or_else(|| panic!("locales/{tag}.json lacks {key}"));
                assert_eq!(
                    placeholders(got),
                    placeholders(text),
                    "locales/{tag}.json {key}: placeholders"
                );
                assert!(!got.trim().is_empty(), "locales/{tag}.json {key} is empty");
            }
            for key in catalog.keys() {
                assert!(
                    en.contains_key(key),
                    "locales/{tag}.json has {key}, which en.json does not"
                );
            }
        }
    }

    #[test]
    fn every_dialog_has_one_distinct_access_key_per_button() {
        let dialogs: &[&[&str]] = &[
            &[keys::CONSENT_ALLOW, keys::CONSENT_BLOCK],
            &[keys::CRASH_RESTART, keys::CRASH_COPY, keys::CRASH_CLOSE],
        ];
        for (tag, catalog) in builtin() {
            for buttons in dialogs {
                let mut seen = Vec::new();
                for key in *buttons {
                    let label = &catalog[*key];
                    let k = mnemonic(label).unwrap_or_else(|| {
                        panic!("locales/{tag}.json {key}: one & before a character: {label:?}")
                    });
                    assert!(
                        !seen.contains(&k),
                        "locales/{tag}.json: access key {k:?} twice in {buttons:?}"
                    );
                    seen.push(k);
                }
            }
        }
    }

    #[test]
    fn negotiation_follows_the_preferences_in_order() {
        let avail = s(&["en", "ru", "pt-BR", "zh-Hans", "zh-Hant", "de"]);
        let n = |p: &[&str]| negotiate(&s(p), &avail);
        assert_eq!(n(&["ru-RU", "en-US"]), "ru");
        assert_eq!(n(&["en-GB"]), "en");
        assert_eq!(n(&["pt-PT"]), "pt-BR", "another variant of the language");
        assert_eq!(n(&["zh-CN"]), "zh-Hans");
        assert_eq!(n(&["zh-TW"]), "zh-Hant");
        assert_eq!(n(&["zh-HK"]), "zh-Hant");
        assert_eq!(n(&["zh"]), "zh-Hans");
        assert_eq!(n(&["zh-Hant-HK"]), "zh-Hant");
        assert_eq!(
            n(&["fy", "de-AT"]),
            "de",
            "an unknown first choice gives way to the next"
        );
        assert_eq!(n(&["xx"]), "en");
        assert_eq!(n(&[]), "en");
    }

    #[test]
    fn placeholders_fill_and_unknown_ones_stay() {
        assert_eq!(
            format("{app} v{version}", &[("app", "Argon"), ("version", "1.2")]),
            "Argon v1.2"
        );
        assert_eq!(format("{a} {b} {", &[("a", "x")]), "x {b} {");
        assert_eq!(format("{a}{a}", &[("a", "y")]), "yy");
    }

    #[test]
    fn the_app_defines_and_chooses_its_language() {
        assert_eq!(
            text_in("ru", keys::CONSENT_ALLOW).as_deref(),
            Some("&Разрешить")
        );
        assert_eq!(
            text_in("ru-RU", keys::CONSENT_ALLOW).as_deref(),
            Some("&Разрешить"),
            "a shorter form"
        );
        assert_eq!(
            text_in("fy", keys::CONSENT_ALLOW).as_deref(),
            Some("&Allow"),
            "English last"
        );
        assert_eq!(text_in("en", "app.unknown"), None);

        define(
            "hy",
            [("vn.consent.allow".to_string(), "&Թույլատրել".to_string())],
        )
        .unwrap();
        define(
            "de",
            [("app.greeting".to_string(), "Hallo {name}".to_string())],
        )
        .unwrap();
        define(
            "ru",
            [("vn.consent.block".to_string(), "&Запретить".to_string())],
        )
        .unwrap();
        assert!(define("not a tag", []).is_err());
        assert!(define_json(r#"{"de":{"a":"b"},"??":{"a":"b"}}"#).is_err());
        assert_eq!(text_in("de", "a"), None, "a bad tag stores nothing");

        assert!(available().iter().any(|t| t == "hy"));
        assert_eq!(
            text_in("hy", keys::CONSENT_ALLOW).as_deref(),
            Some("&Թույլատրել")
        );
        assert_eq!(
            text_in("hy", keys::CONSENT_BLOCK).as_deref(),
            Some("&Block"),
            "missing keys fall back to English"
        );
        assert_eq!(
            text_in("ru", keys::CONSENT_BLOCK).as_deref(),
            Some("&Запретить"),
            "the app's wins"
        );
        assert_eq!(
            text_in("ru", keys::CONSENT_ALLOW).as_deref(),
            Some("&Разрешить"),
            "key by key"
        );

        assert!(set_language(Some("xx_YY.UTF-8")).is_ok());
        assert_eq!(language_override().as_deref(), Some("xx-YY"));
        assert!(set_language(Some("!")).is_err());
        set_language(Some("de-CH")).unwrap();
        assert_eq!(language(), "de");
        assert_eq!(t("app.greeting", &[("name", "Ada")]), "Hallo Ada");
        assert_eq!(app_name(None), "Diese App");
        assert_eq!(app_name(Some(" Argon ")), "Argon");
        let state: serde_json::Value = serde_json::from_str(&state_json()).unwrap();
        assert_eq!(state["language"], "de");
        assert_eq!(state["override"], "de-CH");
        set_language(None).unwrap();
        assert_eq!(language_override(), None);
        assert_eq!(language(), negotiate(&navigator::languages(), &available()));
    }
}
