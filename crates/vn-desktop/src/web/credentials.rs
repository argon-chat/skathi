//! Passkeys: `navigator.credentials` for `publicKey` (WebAuthn Level 3) over the system authenticator.
//! An app has no web origin, so the relying party ID is named by the app (`rp.id`, `rpId`) unless the
//! page is served over https from it; the client data's origin is then `https://<rpId>`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde_json::{json, Value as Json};
use vn_runtime::host_fn;
use vn_runtime::redjs::capi::JSClassRef;
use vn_runtime::redjs::glue;
use vn_runtime::redjs::webapi::util::*;

use super::{abandon, awaiting, global, rejected, rejected_with};
use crate::Fail;

enum Kind {
    Attestation {
        object: Vec<u8>,
        auth_data: Vec<u8>,
        transports: Vec<String>,
        public_key: Option<Vec<u8>>,
        alg: i64,
    },
    Assertion {
        auth_data: Vec<u8>,
        signature: Vec<u8>,
        user_handle: Option<Vec<u8>>,
    },
}

struct Response {
    client_data: Vec<u8>,
    kind: Kind,
}

struct Credential {
    raw: Vec<u8>,
    attachment: Option<String>,
    /// `Some` when `credProps` was asked for: the resident-key answer, if the system gave one.
    cred_props: Option<Option<bool>>,
}

struct Classes {
    cred: JSClassRef,
    resp: JSClassRef,
    pkc: u64,
    attestation: u64,
    assertion: u64,
    s_response: usize,
}

thread_local! {
    static CLASSES: RefCell<Option<Classes>> = const { RefCell::new(None) };
    static CONTAINER: Cell<u64> = const { Cell::new(0) };
    static CONTEXT: RefCell<HashMap<u32, (Vec<u8>, bool)>> = RefCell::new(HashMap::new());
}

fn with<R>(f: impl FnOnce(&Classes) -> R) -> R {
    CLASSES.with(|c| f(c.borrow().as_ref().expect("credentials installed")))
}

unsafe fn credential<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Credential> {
    host_data::<Credential>(g, with(|c| c.cred), v)
}

unsafe fn response<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Response> {
    host_data::<Response>(g, with(|c| c.resp), v)
}

unsafe fn buffer(g: *mut c_void, bytes: &[u8]) -> u64 {
    get(g, make_u8(g, bytes), "buffer")
}

fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    B64.decode(s.trim_end_matches('=').replace('+', "-").replace('/', "_"))
        .ok()
}

fn field(v: &Json, k: &str) -> Vec<u8> {
    v[k].as_str().and_then(unb64).unwrap_or_default()
}

fn valid_domain(d: &str) -> bool {
    let labels_ok = d.split('.').all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let ipv4 = d.split('.').count() == 4
        && d.split('.')
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()));
    !d.is_empty() && d.len() <= 253 && labels_ok && !ipv4
}

/// `(host, port)` of an https page URL.
fn https_host(url: &str) -> Option<(String, String)> {
    let rest = url
        .get(..8)
        .filter(|p| p.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) => (h, format!(":{p}")),
        _ => (authority, String::new()),
    };
    (!host.is_empty()).then(|| (host.to_ascii_lowercase(), port))
}

/// `(rpId, origin)` for the client data, or the SecurityError's message.
pub(super) fn relying_party(
    page: &str,
    requested: Option<&str>,
) -> Result<(String, String), String> {
    let page = https_host(page);
    let rp_id = match requested {
        Some(r) => r.to_ascii_lowercase(),
        None => page.as_ref().map(|(h, _)| h.clone()).unwrap_or_default(),
    };
    if rp_id.is_empty() {
        return Err("This app has no web origin: name the relying party with 'rp.id' (create) or 'rpId' (get).".into());
    }
    if !valid_domain(&rp_id) {
        return Err(format!("'{rp_id}' is an invalid domain."));
    }
    let origin = match page {
        Some((host, port)) if host == rp_id || host.ends_with(&format!(".{rp_id}")) => {
            format!("https://{host}{port}")
        }
        _ => format!("https://{rp_id}"),
    };
    Ok((rp_id, origin))
}

pub(super) fn client_data(ty: &str, challenge: &[u8], origin: &str) -> String {
    let s = |v: &str| serde_json::to_string(v).unwrap_or_default();
    format!(
        r#"{{"type":{},"challenge":{},"origin":{},"crossOrigin":false}}"#,
        s(ty),
        s(&b64(challenge)),
        s(origin)
    )
}

fn timeout(n: Option<f64>) -> u64 {
    match n {
        Some(n) if n.is_finite() && n > 0.0 => n.clamp(10_000.0, 600_000.0) as u64,
        _ => 300_000,
    }
}

unsafe fn page_url(g: *mut c_void) -> String {
    let location = get(g, global(g), "location");
    if is_cell(location) {
        read_string(g, get(g, location, "href")).unwrap_or_default()
    } else {
        String::new()
    }
}

unsafe fn member(g: *mut c_void, obj: u64, k: &str) -> u64 {
    if is_cell(obj) {
        get(g, obj, k)
    } else {
        UNDEF
    }
}

unsafe fn string(g: *mut c_void, v: u64) -> Option<String> {
    if v == UNDEF {
        return None;
    }
    to_js_string(g, v).ok().and_then(|s| read_string(g, s))
}

unsafe fn list(g: *mut c_void, v: u64) -> Vec<u64> {
    if !is_cell(v) {
        return Vec::new();
    }
    let Ok(arr) = call(g, get(g, get(g, global(g), "Array"), "from"), UNDEF, &[v]) else {
        return Vec::new();
    };
    (0..array_len(g, arr)).map(|i| get_i(g, arr, i)).collect()
}

unsafe fn descriptors(g: *mut c_void, v: u64) -> Result<Json, String> {
    let mut out = Vec::new();
    for d in list(g, v) {
        if string(g, member(g, d, "type")).as_deref() != Some("public-key") {
            continue;
        }
        let Some(id) = bufferish(g, member(g, d, "id")) else {
            return Err("A credential descriptor's 'id' is not a BufferSource.".into());
        };
        let transports: Vec<String> = list(g, member(g, d, "transports"))
            .into_iter()
            .filter_map(|t| string(g, t))
            .collect();
        out.push(json!({ "id": b64(id), "transports": transports }));
    }
    Ok(Json::Array(out))
}

#[cfg(not(test))]
fn start(create: bool, req: Json) -> Result<u32, Fail> {
    let Some(owner) = crate::dialog::current_owner() else {
        return Err(Fail::new(
            "NotAllowedError",
            "There is no window to ask from.",
        ));
    };
    if create {
        crate::webauthn::start_create(owner.hwnd, req)
    } else {
        crate::webauthn::start_get(owner.hwnd, req)
    }
}

#[cfg(test)]
fn start(create: bool, req: Json) -> Result<u32, Fail> {
    tests::fake(create, req)
}

host_fn!(fn on_abort(g, args) {
    unsafe {
        let token = args.arg(0).as_number().unwrap_or(0.0) as u32;
        let signal = args.arg(1).0;
        let reason = get(g, get(g, signal, "currentTarget"), "reason");
        crate::webauthn::cancel(token);
        abandon(g, token, reason);
    }
    UNDEF
});

unsafe fn watch(g: *mut c_void, signal: u64, token: u32) {
    if !is_cell(signal) {
        return;
    }
    let f = make_fn(g, "", 1, on_abort);
    let Ok(bound) = call(g, get(g, f, "bind"), f, &[UNDEF, enc_f64(token as f64)]) else {
        return;
    };
    let opts = new_object(g, UNDEF);
    define(g, opts, "once", TRUE, 0);
    let _ = call(
        g,
        get(g, signal, "addEventListener"),
        signal,
        &[js_string(g, "abort"), bound, opts],
    );
}

unsafe fn make_credential(
    g: *mut c_void,
    raw: Vec<u8>,
    attachment: Option<String>,
    cred_props: Option<Option<bool>>,
    resp: Response,
) -> u64 {
    let attestation = matches!(resp.kind, Kind::Attestation { .. });
    let (cred_cls, resp_cls, pkc, att, asr, slot) = with(|c| {
        (
            c.cred,
            c.resp,
            c.pkc,
            c.attestation,
            c.assertion,
            c.s_response,
        )
    });
    let r = host_create(g, resp_cls, if attestation { att } else { asr }, resp);
    let obj = host_create(
        g,
        cred_cls,
        pkc,
        Credential {
            raw,
            attachment,
            cred_props,
        },
    );
    glue::rjs_define(g, obj, slot as *mut c_void, r, 0);
    obj
}

/// What was signed: the system's own client data when it wrote one, else the one sent with the request.
fn signed_client_data(v: &Json, sent: Vec<u8>) -> Vec<u8> {
    v["clientDataJSON"].as_str().and_then(unb64).unwrap_or(sent)
}

unsafe fn created(g: *mut c_void, token: u32, v: &Json, _made: u64) -> Result<u64, u64> {
    let (client_data, cred_props) = CONTEXT
        .with(|c| c.borrow_mut().remove(&token))
        .unwrap_or_default();
    let resp = Response {
        client_data: signed_client_data(v, client_data),
        kind: Kind::Attestation {
            object: field(v, "attestationObject"),
            auth_data: field(v, "authenticatorData"),
            transports: v["transports"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
            public_key: v["publicKey"].as_str().and_then(unb64),
            alg: v["publicKeyAlgorithm"].as_i64().unwrap_or(0),
        },
    };
    let props = cred_props.then(|| v["residentKey"].as_bool());
    Ok(make_credential(
        g,
        field(v, "id"),
        v["attachment"].as_str().map(str::to_string),
        props,
        resp,
    ))
}

unsafe fn asserted(g: *mut c_void, token: u32, v: &Json, _made: u64) -> Result<u64, u64> {
    let (client_data, _) = CONTEXT
        .with(|c| c.borrow_mut().remove(&token))
        .unwrap_or_default();
    let resp = Response {
        client_data: signed_client_data(v, client_data),
        kind: Kind::Assertion {
            auth_data: field(v, "authenticatorData"),
            signature: field(v, "signature"),
            user_handle: v["userHandle"].as_str().and_then(unb64),
        },
    };
    Ok(make_credential(
        g,
        field(v, "id"),
        v["attachment"].as_str().map(str::to_string),
        None,
        resp,
    ))
}

unsafe fn begin(
    g: *mut c_void,
    create: bool,
    signal: u64,
    client_data: String,
    cred_props: bool,
    req: Json,
) -> u64 {
    let started = start(create, req);
    if let Ok(token) = started {
        CONTEXT.with(|c| {
            c.borrow_mut()
                .insert(token, (client_data.into_bytes(), cred_props))
        });
        let p = awaiting(g, started, if create { created } else { asserted });
        watch(g, signal, token);
        return p;
    }
    awaiting(g, started, created)
}

unsafe fn options_signal(g: *mut c_void, options: u64) -> Result<u64, u64> {
    let signal = member(g, options, "signal");
    if is_cell(signal) && get(g, signal, "aborted") == TRUE {
        return Err(rejected_with(g, get(g, signal, "reason")));
    }
    Ok(signal)
}

host_fn!(fn container_create(g, args) {
    unsafe {
        let options = args.arg(0).0;
        let signal = match options_signal(g, options) { Ok(s) => s, Err(p) => return p };
        let pk = member(g, options, "publicKey");
        if !is_cell(pk) {
            return rejected(g, "NotSupportedError", "Only 'publicKey' credentials are supported.");
        }
        let type_error = |m: &str| rejected(g, "TypeError", &format!("Failed to execute 'create' on 'CredentialsContainer': {m}"));
        let (rp, user) = (member(g, pk, "rp"), member(g, pk, "user"));
        let Some(rp_name) = string(g, member(g, rp, "name")) else { return type_error("Required member is undefined: rp.name") };
        let (Some(name), Some(display)) = (string(g, member(g, user, "name")), string(g, member(g, user, "displayName"))) else {
            return type_error("Required member is undefined: user.name or user.displayName");
        };
        let Some(user_id) = bufferish(g, member(g, user, "id")).map(<[u8]>::to_vec) else { return type_error("user.id is not a BufferSource") };
        if user_id.is_empty() || user_id.len() > 64 {
            return type_error("user.id must be between 1 and 64 bytes.");
        }
        let Some(challenge) = bufferish(g, member(g, pk, "challenge")).map(<[u8]>::to_vec) else { return type_error("challenge is not a BufferSource") };
        let params = list(g, member(g, pk, "pubKeyCredParams"));
        let mut algs: Vec<i64> = params
            .iter()
            .filter(|p| string(g, member(g, **p, "type")).as_deref() == Some("public-key"))
            .filter_map(|p| dec_f64(member(g, *p, "alg")).map(|a| a as i64))
            .collect();
        if !params.is_empty() && algs.is_empty() {
            return rejected(g, "NotSupportedError", "None of the requested credential types are supported.");
        }
        if algs.is_empty() {
            algs = vec![-7, -257];
        }
        let (rp_id, origin) = match relying_party(&page_url(g), string(g, member(g, rp, "id")).as_deref()) {
            Ok(r) => r,
            Err(m) => return rejected(g, "SecurityError", &m),
        };
        let sel = member(g, pk, "authenticatorSelection");
        let resident = string(g, member(g, sel, "residentKey")).unwrap_or_else(|| if member(g, sel, "requireResidentKey") == TRUE { "required".into() } else { "discouraged".into() });
        let exclude = match descriptors(g, member(g, pk, "excludeCredentials")) { Ok(d) => d, Err(m) => return type_error(&m) };
        let cred_props = member(g, member(g, pk, "extensions"), "credProps") == TRUE;
        let cd = client_data("webauthn.create", &challenge, &origin);
        let req = json!({
            "rpId": rp_id,
            "challenge": b64(&challenge),
            "rpName": rp_name,
            "userId": b64(&user_id),
            "userName": name,
            "userDisplayName": display,
            "clientDataJSON": cd,
            "algs": algs,
            "timeout": timeout(dec_f64(member(g, pk, "timeout"))),
            "exclude": exclude,
            "attachment": string(g, member(g, sel, "authenticatorAttachment")).unwrap_or_default(),
            "residentKey": resident,
            "userVerification": string(g, member(g, sel, "userVerification")).unwrap_or_else(|| "preferred".into()),
            "attestation": string(g, member(g, pk, "attestation")).unwrap_or_else(|| "none".into()),
        });
        begin(g, true, signal, cd, cred_props, req)
    }
});

host_fn!(fn container_get(g, args) {
    unsafe {
        let options = args.arg(0).0;
        let signal = match options_signal(g, options) { Ok(s) => s, Err(p) => return p };
        let pk = member(g, options, "publicKey");
        if !is_cell(pk) {
            return rejected(g, "NotSupportedError", "Only 'publicKey' credentials are supported.");
        }
        if string(g, member(g, options, "mediation")).as_deref() == Some("conditional") {
            return rejected(g, "NotSupportedError", "Conditional mediation is not supported.");
        }
        let Some(challenge) = bufferish(g, member(g, pk, "challenge")).map(<[u8]>::to_vec) else {
            return rejected(g, "TypeError", "Failed to execute 'get' on 'CredentialsContainer': challenge is not a BufferSource");
        };
        let (rp_id, origin) = match relying_party(&page_url(g), string(g, member(g, pk, "rpId")).as_deref()) {
            Ok(r) => r,
            Err(m) => return rejected(g, "SecurityError", &m),
        };
        let allow = match descriptors(g, member(g, pk, "allowCredentials")) {
            Ok(d) => d,
            Err(m) => return rejected(g, "TypeError", &format!("Failed to execute 'get' on 'CredentialsContainer': {m}")),
        };
        let cd = client_data("webauthn.get", &challenge, &origin);
        let req = json!({
            "rpId": rp_id,
            "challenge": b64(&challenge),
            "clientDataJSON": cd,
            "timeout": timeout(dec_f64(member(g, pk, "timeout"))),
            "allow": allow,
            "userVerification": string(g, member(g, pk, "userVerification")).unwrap_or_else(|| "preferred".into()),
        });
        begin(g, false, signal, cd, false, req)
    }
});

host_fn!(fn container_store(g, _args) {
    unsafe { rejected(g, "NotSupportedError", "Only 'publicKey' credentials are supported.") }
});

host_fn!(fn container_prevent_silent_access(g, _args) {
    unsafe { resolved_promise(g, UNDEF) }
});

macro_rules! cred_getter {
    ($name:ident, |$g:ident, $c:ident, $this:ident| $body:expr) => {
        host_fn!(fn $name(g, args) {
            unsafe {
                let $this = args.this().0;
                let $g = g;
                match credential(g, $this) {
                    Some($c) => $body,
                    None => throw_type_error(g, "Illegal invocation"),
                }
            }
        });
    };
}

cred_getter!(cred_id, |g, c, _this| js_string(g, &b64(&c.raw)));
cred_getter!(cred_type, |g, _c, _this| js_string(g, "public-key"));
cred_getter!(cred_raw_id, |g, c, _this| buffer(g, &c.raw));
cred_getter!(cred_attachment, |g, c, _this| c
    .attachment
    .as_deref()
    .map_or(NULL, |a| js_string(g, a)));
cred_getter!(cred_response, |g, _c, this| get_a(
    g,
    this,
    with(|c| c.s_response) as *mut c_void
));
cred_getter!(cred_extensions, |g, c, _this| extensions(g, c));

unsafe fn extensions(g: *mut c_void, c: &Credential) -> u64 {
    let out = new_object(g, UNDEF);
    if let Some(rk) = c.cred_props {
        let props = new_object(g, UNDEF);
        if let Some(rk) = rk {
            define(g, props, "rk", enc_bool(rk), 0);
        }
        define(g, out, "credProps", props, 0);
    }
    out
}

unsafe fn put(g: *mut c_void, obj: u64, k: &str, bytes: &[u8]) {
    define(g, obj, k, js_string(g, &b64(bytes)), 0);
}

host_fn!(fn cred_to_json(g, args) {
    unsafe {
        let this = args.this().0;
        let Some(c) = credential(g, this) else { return throw_type_error(g, "Illegal invocation") };
        let r = get_a(g, this, with(|c| c.s_response) as *mut c_void);
        let Some(resp) = response(g, r) else { return throw_type_error(g, "Illegal invocation") };
        let out = new_object(g, UNDEF);
        let id = b64(&c.raw);
        define(g, out, "id", js_string(g, &id), 0);
        define(g, out, "rawId", js_string(g, &id), 0);
        define(g, out, "type", js_string(g, "public-key"), 0);
        define(g, out, "authenticatorAttachment", c.attachment.as_deref().map_or(NULL, |a| js_string(g, a)), 0);
        define(g, out, "clientExtensionResults", extensions(g, c), 0);
        let ro = new_object(g, UNDEF);
        put(g, ro, "clientDataJSON", &resp.client_data);
        match &resp.kind {
            Kind::Attestation { object, auth_data, transports, public_key, alg } => {
                put(g, ro, "attestationObject", object);
                put(g, ro, "authenticatorData", auth_data);
                define(g, ro, "transports", sorted_transports(g, transports), 0);
                if let Some(pk) = public_key {
                    put(g, ro, "publicKey", pk);
                }
                define(g, ro, "publicKeyAlgorithm", enc_f64(*alg as f64), 0);
            }
            Kind::Assertion { auth_data, signature, user_handle } => {
                put(g, ro, "authenticatorData", auth_data);
                put(g, ro, "signature", signature);
                if let Some(u) = user_handle {
                    put(g, ro, "userHandle", u);
                }
            }
        }
        define(g, out, "response", ro, 0);
        out
    }
});

unsafe fn sorted_transports(g: *mut c_void, t: &[String]) -> u64 {
    let mut t = t.to_vec();
    t.sort();
    let arr = new_array(g);
    for (i, s) in t.iter().enumerate() {
        set_i(g, arr, i as u32, js_string(g, s));
    }
    arr
}

macro_rules! resp_getter {
    ($name:ident, |$g:ident, $r:ident| $body:expr) => {
        host_fn!(fn $name(g, args) {
            unsafe {
                let $g = g;
                match response(g, args.this().0) {
                    Some($r) => $body,
                    None => throw_type_error(g, "Illegal invocation"),
                }
            }
        });
    };
}

resp_getter!(resp_client_data, |g, r| buffer(g, &r.client_data));
resp_getter!(resp_attestation_object, |g, r| match &r.kind {
    Kind::Attestation { object, .. } => buffer(g, object),
    _ => throw_type_error(g, "Illegal invocation"),
});
resp_getter!(resp_transports, |g, r| match &r.kind {
    Kind::Attestation { transports, .. } => sorted_transports(g, transports),
    _ => throw_type_error(g, "Illegal invocation"),
});
resp_getter!(resp_authenticator_data, |g, r| match &r.kind {
    Kind::Attestation { auth_data, .. } | Kind::Assertion { auth_data, .. } => buffer(g, auth_data),
});
resp_getter!(resp_public_key, |g, r| match &r.kind {
    Kind::Attestation { public_key, .. } => public_key.as_deref().map_or(NULL, |k| buffer(g, k)),
    _ => throw_type_error(g, "Illegal invocation"),
});
resp_getter!(resp_public_key_algorithm, |g, r| match &r.kind {
    Kind::Attestation { alg, .. } => enc_f64(*alg as f64),
    _ => throw_type_error(g, "Illegal invocation"),
});
resp_getter!(resp_signature, |g, r| match &r.kind {
    Kind::Assertion { signature, .. } => buffer(g, signature),
    _ => throw_type_error(g, "Illegal invocation"),
});
resp_getter!(resp_user_handle, |g, r| match &r.kind {
    Kind::Assertion { user_handle, .. } => user_handle.as_deref().map_or(NULL, |u| buffer(g, u)),
    _ => throw_type_error(g, "Illegal invocation"),
});

host_fn!(fn uvpa_available(g, _args) {
    unsafe { resolved_promise(g, enc_bool(crate::webauthn::available())) }
});

host_fn!(fn conditional_available(g, _args) {
    unsafe { resolved_promise(g, FALSE) }
});

host_fn!(fn client_capabilities(g, _args) {
    unsafe {
        let uvpa = crate::webauthn::available();
        let out = new_object(g, UNDEF);
        for (k, v) in [
            ("conditionalCreate", false),
            ("conditionalGet", false),
            ("hybridTransport", true),
            ("passkeyPlatformAuthenticator", uvpa),
            ("userVerifyingPlatformAuthenticator", uvpa),
            ("relatedOrigins", false),
            ("signalAllAcceptedCredentials", false),
            ("signalCurrentUserDetails", false),
            ("signalUnknownCredential", false),
            ("extension:credProps", true),
        ] {
            define(g, out, k, enc_bool(v), 0);
        }
        resolved_promise(g, out)
    }
});

/// A shallow copy of `options` with the base64url members `keys` decoded, or a TypeError thrown.
unsafe fn decode_members(g: *mut c_void, options: u64, keys: &[&str]) -> Result<u64, ()> {
    let object = get(g, global(g), "Object");
    let copy = call(
        g,
        get(g, object, "assign"),
        object,
        &[new_object(g, UNDEF), options],
    )
    .map_err(|_| ())?;
    for k in keys {
        let v = member(g, copy, k);
        let Some(bytes) = string(g, v).and_then(|s| unb64(&s)).filter(|_| v != UNDEF) else {
            throw_type_error(
                g,
                &format!("Failed to read the '{k}' property: it is not valid base64url."),
            );
            return Err(());
        };
        define(g, copy, k, buffer(g, &bytes), 0);
    }
    Ok(copy)
}

unsafe fn decode_list(g: *mut c_void, obj: u64, key: &str) -> Result<(), ()> {
    let v = member(g, obj, key);
    if !is_cell(v) {
        return Ok(());
    }
    let arr = new_array(g);
    for (i, d) in list(g, v).into_iter().enumerate() {
        set_i(g, arr, i as u32, decode_members(g, d, &["id"])?);
    }
    define(g, obj, key, arr, 0);
    Ok(())
}

host_fn!(fn parse_creation_options(g, args) {
    unsafe {
        let Ok(copy) = decode_members(g, args.arg(0).0, &["challenge"]) else { return UNDEF };
        let user = member(g, copy, "user");
        let Ok(user) = decode_members(g, user, &["id"]) else { return UNDEF };
        define(g, copy, "user", user, 0);
        if decode_list(g, copy, "excludeCredentials").is_err() {
            return UNDEF;
        }
        copy
    }
});

host_fn!(fn parse_request_options(g, args) {
    unsafe {
        let Ok(copy) = decode_members(g, args.arg(0).0, &["challenge"]) else { return UNDEF };
        if decode_list(g, copy, "allowCredentials").is_err() {
            return UNDEF;
        }
        copy
    }
});

host_fn!(fn navigator_credentials(_g, _args) {
    CONTAINER.with(|c| c.get())
});

pub(super) unsafe fn install(g: *mut c_void, nav: u64) {
    let cred = make_host_class::<Credential>("PublicKeyCredential");
    let resp = make_host_class::<Response>("AuthenticatorResponse");
    let base = define_class(
        g,
        &ClassSpec {
            name: "Credential",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[],
            getters: &[("id", cred_id), ("type", cred_type)],
            accessors: &[],
        },
    );
    let pkc = define_class(
        g,
        &ClassSpec {
            name: "PublicKeyCredential",
            arity: 0,
            construct: None,
            parent_proto: base.proto,
            methods: &[
                ("getClientExtensionResults", 0, cred_extensions),
                ("toJSON", 0, cred_to_json),
            ],
            getters: &[
                ("rawId", cred_raw_id),
                ("response", cred_response),
                ("authenticatorAttachment", cred_attachment),
            ],
            accessors: &[],
        },
    );
    for (name, arity, f) in [
        (
            "isUserVerifyingPlatformAuthenticatorAvailable",
            0,
            uvpa_available as HostFn,
        ),
        ("isConditionalMediationAvailable", 0, conditional_available),
        ("getClientCapabilities", 0, client_capabilities),
        ("parseCreationOptionsFromJSON", 1, parse_creation_options),
        ("parseRequestOptionsFromJSON", 1, parse_request_options),
    ] {
        define(g, pkc.ctor, name, make_fn(g, name, arity, f), 2);
    }
    let response_base = define_class(
        g,
        &ClassSpec {
            name: "AuthenticatorResponse",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[],
            getters: &[("clientDataJSON", resp_client_data)],
            accessors: &[],
        },
    );
    let attestation = define_class(
        g,
        &ClassSpec {
            name: "AuthenticatorAttestationResponse",
            arity: 0,
            construct: None,
            parent_proto: response_base.proto,
            methods: &[
                ("getTransports", 0, resp_transports),
                ("getAuthenticatorData", 0, resp_authenticator_data),
                ("getPublicKey", 0, resp_public_key),
                ("getPublicKeyAlgorithm", 0, resp_public_key_algorithm),
            ],
            getters: &[("attestationObject", resp_attestation_object)],
            accessors: &[],
        },
    );
    let assertion = define_class(
        g,
        &ClassSpec {
            name: "AuthenticatorAssertionResponse",
            arity: 0,
            construct: None,
            parent_proto: response_base.proto,
            methods: &[],
            getters: &[
                ("authenticatorData", resp_authenticator_data),
                ("signature", resp_signature),
                ("userHandle", resp_user_handle),
            ],
            accessors: &[],
        },
    );
    let container = define_class(
        g,
        &ClassSpec {
            name: "CredentialsContainer",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("create", 0, container_create),
                ("get", 0, container_get),
                ("store", 1, container_store),
                ("preventSilentAccess", 0, container_prevent_silent_access),
            ],
            getters: &[],
            accessors: &[],
        },
    );
    let instance = new_object(g, container.proto);
    vn_runtime::redjs::ffi::redjs_protect(g, instance);
    CONTAINER.with(|c| c.set(instance));
    CLASSES.with(|c| {
        *c.borrow_mut() = Some(Classes {
            cred,
            resp,
            pkc: pkc.proto,
            attestation: attestation.proto,
            assertion: assertion.proto,
            s_response: atom_priv(g, "vn#response") as usize,
        })
    });
    super::define_getter(g, nav, "credentials", navigator_credentials);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static REQUESTS: Mutex<Vec<Json>> = Mutex::new(Vec::new());
    static HANG: Mutex<bool> = Mutex::new(false);

    pub(super) fn fake(create: bool, req: Json) -> Result<u32, Fail> {
        let token = crate::next_token();
        REQUESTS.lock().unwrap().push(req);
        if *HANG.lock().unwrap() {
            return Ok(token);
        }
        let e = |s: &str| b64(s.as_bytes());
        let payload = if create {
            json!({ "id": e("cred"), "attestationObject": e("att"), "authenticatorData": e("auth"), "transports": ["internal", "hybrid"], "attachment": "platform", "publicKey": e("spki"), "publicKeyAlgorithm": -7, "residentKey": true })
        } else {
            json!({ "id": e("cred"), "authenticatorData": e("auth"), "signature": e("sig"), "userHandle": e("u1"), "attachment": "platform", "clientDataJSON": e("sys") })
        };
        crate::settle(token, Ok(payload.to_string()));
        Ok(token)
    }

    #[test]
    fn the_relying_party_and_origin() {
        assert_eq!(
            relying_party("app://redjs/i.html", Some("Example.com")),
            Ok(("example.com".into(), "https://example.com".into()))
        );
        assert_eq!(
            relying_party("https://login.example.com:8443/x", Some("example.com"))
                .unwrap()
                .1,
            "https://login.example.com:8443"
        );
        assert_eq!(
            relying_party("https://login.example.com/x", None).unwrap(),
            (
                "login.example.com".into(),
                "https://login.example.com".into()
            )
        );
        assert!(relying_party("app://redjs/", None).is_err());
        assert!(relying_party("app://redjs/", Some("bad domain")).is_err());
        assert!(relying_party("app://redjs/", Some("1.2.3.4")).is_err());
        assert_eq!(
            client_data("webauthn.get", &[1, 2, 3], "https://a.b"),
            r#"{"type":"webauthn.get","challenge":"AQID","origin":"https://a.b","crossOrigin":false}"#
        );
    }

    #[test]
    fn create_and_get_build_credentials_and_abort_rejects() {
        let out = super::super::testing::run(
            "const enc = (s) => new TextEncoder().encode(s); const dec = (b) => new TextDecoder().decode(b); \
             const c = await navigator.credentials.create({ publicKey: { rp: { id: 'example.com', name: 'Ex' }, user: { id: enc('u1'), name: 'ann', displayName: 'Ann' }, \
               challenge: new Uint8Array([1, 2, 3]), pubKeyCredParams: [{ type: 'public-key', alg: -7 }], authenticatorSelection: { residentKey: 'required' }, extensions: { credProps: true } } }); \
             const r = [c instanceof PublicKeyCredential, c instanceof Credential, c.type, dec(c.rawId), c.authenticatorAttachment, \
               c.response instanceof AuthenticatorAttestationResponse, c.response.getTransports().join('+'), dec(c.response.getPublicKey()), \
               JSON.parse(dec(c.response.clientDataJSON)).origin, JSON.stringify(c.getClientExtensionResults()), c.toJSON().response.publicKeyAlgorithm]; \
             const a = await navigator.credentials.get({ publicKey: { challenge: new Uint8Array([9]), rpId: 'example.com', allowCredentials: [{ type: 'public-key', id: new Uint8Array([5]) }] } }); \
             r.push(a.response instanceof AuthenticatorAssertionResponse, dec(a.response.signature), dec(a.response.userHandle), a.toJSON().response.userHandle, dec(a.response.clientDataJSON)); \
             for (const bad of [{}, { publicKey: { challenge: new Uint8Array([1]) } }]) { try { await navigator.credentials.get(bad); } catch (e) { r.push(e.name); } } \
             const o = PublicKeyCredential.parseRequestOptionsFromJSON({ challenge: 'AQID', allowCredentials: [{ type: 'public-key', id: 'BQ' }] }); \
             r.push(new Uint8Array(o.challenge).join('.'), new Uint8Array(o.allowCredentials[0].id)[0]); \
             try { PublicKeyCredential.parseRequestOptionsFromJSON({ challenge: '!!' }); } catch (e) { r.push(e instanceof TypeError); } \
             return r.join('|');",
        );
        assert_eq!(
            out,
            "true|true|public-key|cred|platform|true|hybrid+internal|spki|https://example.com|{\"credProps\":{\"rk\":true}}|-7|true|sig|u1|dTE|sys|NotSupportedError|SecurityError|1.2.3|5|true"
        );
        let reqs = REQUESTS.lock().unwrap().drain(..).collect::<Vec<_>>();
        assert_eq!(reqs[0]["rpId"], "example.com");
        assert_eq!(reqs[0]["algs"], json!([-7]));
        assert_eq!((reqs[0]["challenge"].as_str(), reqs[1]["challenge"].as_str()), (Some("AQID"), Some("CQ")));
        assert_eq!(reqs[1]["allow"], json!([{ "id": "BQ", "transports": [] }]));

        *HANG.lock().unwrap() = true;
        let out = super::super::testing::run(
            "const ac = new AbortController(); \
             const p = navigator.credentials.get({ publicKey: { challenge: new Uint8Array([1]), rpId: 'example.com' }, signal: ac.signal }); \
             ac.abort(new Error('stop')); try { await p; return 'resolved'; } catch (e) { return e.message; }",
        );
        *HANG.lock().unwrap() = false;
        assert_eq!(out, "stop");
    }
}
