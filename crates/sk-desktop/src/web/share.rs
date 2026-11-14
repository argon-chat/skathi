//! `navigator.share` / `navigator.canShare` through the system share sheet.

use std::cell::Cell;
use std::ffi::c_void;

use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::{blob, util::*};

use super::{awaiting, global, is_pending, rejected};
use crate::share::{Request, SharedFile};

thread_local! {
    static LAST: Cell<u32> = const { Cell::new(0) };
}

struct Checked {
    title: Option<String>,
    text: Option<String>,
    url: Option<String>,
    files: Vec<u64>,
}

unsafe fn text(g: *mut c_void, v: u64) -> Option<Option<String>> {
    if v == UNDEF {
        return Some(None);
    }
    Some(Some(
        read_string(g, to_js_string(g, v).ok()?).unwrap_or_default(),
    ))
}

/// The share data, or the TypeError's message. `None`: a conversion threw.
unsafe fn check(g: *mut c_void, data: u64) -> Option<Result<Checked, String>> {
    if data != UNDEF && data != NULL && !is_cell(data) {
        return Some(Err("The provided value is not of type 'ShareData'.".into()));
    }
    let member = |k: &str| {
        if is_cell(data) {
            try_get(g, data, k).ok()
        } else {
            Some(UNDEF)
        }
    };
    let title = text(g, member("title")?)?;
    let body = text(g, member("text")?)?;
    let url = match text(g, member("url")?)? {
        None => None,
        Some(raw) => {
            let reflect = get(g, global(g), "Reflect");
            let args = new_array(g);
            set_i(g, args, 0, js_string(g, &raw));
            let location = get(g, global(g), "location");
            if is_cell(location) {
                set_i(g, args, 1, get(g, location, "href"));
            }
            match call(
                g,
                get(g, reflect, "construct"),
                reflect,
                &[get(g, global(g), "URL"), args],
            ) {
                Ok(u) => Some(read_string(g, get(g, u, "href")).unwrap_or_default()),
                Err(_) => return Some(Err("Invalid URL".into())),
            }
        }
    };
    let mut files = Vec::new();
    let list = member("files")?;
    if list != UNDEF {
        let Ok(arr) = call(
            g,
            get(g, get(g, global(g), "Array"), "from"),
            UNDEF,
            &[list],
        ) else {
            return Some(Err(
                "The provided value cannot be converted to a sequence.".into()
            ));
        };
        let file_proto = get(g, get(g, global(g), "File"), "prototype");
        for i in 0..array_len(g, arr) {
            let f = get_i(g, arr, i);
            if !is_cell(f) || !proto_chain_has(g, f, file_proto) {
                return Some(Err("Failed to convert value to 'File'.".into()));
            }
            files.push(f);
        }
    }
    if title.is_none() && body.is_none() && url.is_none() && files.is_empty() {
        return Some(Err(
            "No known share data fields supplied. If using only new fields (other than title, text and url), you must feature-detect them first.".into(),
        ));
    }
    Some(Ok(Checked {
        title,
        text: body,
        url,
        files,
    }))
}

unsafe fn user_active(g: *mut c_void) -> bool {
    let ua = get(g, get(g, global(g), "navigator"), "userActivation");
    is_cell(ua) && get(g, ua, "isActive") == TRUE
}

unsafe fn shared(_g: *mut c_void, _token: u32, _v: &serde_json::Value, _made: u64) -> Result<u64, u64> {
    Ok(UNDEF)
}

host_fn!(fn share(g, args) {
    unsafe {
        if is_pending(LAST.with(|l| l.get())) {
            return rejected(g, "InvalidStateError", "An earlier share has not yet completed.");
        }
        if !user_active(g) {
            return rejected(g, "NotAllowedError", "Must be handling a user gesture to perform a share request.");
        }
        let c = match check(g, args.arg(0).0) {
            None => return rejected(g, "TypeError", "Failed to execute 'share' on 'Navigator': the share data could not be read."),
            Some(Err(m)) => return rejected(g, "TypeError", &format!("Failed to execute 'share' on 'Navigator': {m}")),
            Some(Ok(c)) => c,
        };
        let mut files = Vec::new();
        for f in &c.files {
            let name = read_string(g, get(g, *f, "name")).unwrap_or_default();
            match blob::bytes_of(g, *f, "navigator.share") {
                Some(Ok(bytes)) => files.push(SharedFile { name, bytes }),
                _ => return rejected(g, "NotReadableError", &format!("The file '{name}' could not be read.")),
            }
        }
        let doc = get(g, global(g), "document");
        let doc_title = if is_cell(doc) { read_string(g, get(g, doc, "title")).unwrap_or_default() } else { String::new() };
        let title = c.title.filter(|t| !t.is_empty()).or_else(|| Some(doc_title).filter(|t| !t.is_empty())).or_else(|| c.url.clone()).unwrap_or_default();
        let Some(owner) = crate::dialog::current_owner() else {
            return rejected(g, "NotAllowedError", "There is no window to share from.");
        };
        let started = crate::share::start(owner.hwnd, Request { title, text: c.text, url: c.url, files });
        if let Ok(t) = started {
            LAST.with(|l| l.set(t));
        }
        awaiting(g, started, shared)
    }
});

host_fn!(fn can_share(g, args) {
    unsafe { enc_bool(matches!(check(g, args.arg(0).0), Some(Ok(_)))) }
});

pub(super) unsafe fn install(g: *mut c_void, nav: u64) {
    define(g, nav, "share", make_fn(g, "share", 0, share), 2);
    define(g, nav, "canShare", make_fn(g, "canShare", 0, can_share), 2);
}

#[cfg(test)]
mod tests {
    #[test]
    fn can_share_checks_what_share_would_refuse() {
        let out = super::super::testing::run(
            "const c = (d) => navigator.canShare(d); \
             const r = [c({ title: 't' }), c({}), c(), c({ url: 'http://[' }), c({ files: ['x'] }), c({ files: [] }), c({ files: [new File(['a'], 'a.txt')] })]; \
             try { await navigator.share({ title: 't' }); } catch (e) { r.push(e.name); } \
             Object.defineProperty(navigator, 'userActivation', { value: { isActive: true }, configurable: true }); \
             try { await navigator.share({}); } catch (e) { r.push(e instanceof TypeError); } \
             delete navigator.userActivation; \
             return r.join();",
        );
        assert_eq!(
            out,
            "true,false,false,false,false,false,true,NotAllowedError,true"
        );
    }
}
