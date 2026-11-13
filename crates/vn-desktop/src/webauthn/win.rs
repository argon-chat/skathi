//! webauthn.dll: Windows Hello, security keys and phones over hybrid. It signs the client data it is
//! given.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde_json::{json, Value};
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Networking::WindowsWebServices::*;

use super::{cose_alg, cose_public_key};
use crate::Fail;

static CANCEL: Mutex<Option<HashMap<u32, GUID>>> = Mutex::new(None);

struct Wide(Vec<u16>);

impl Wide {
    fn new(s: &str) -> Self {
        Wide(s.encode_utf16().chain([0]).collect())
    }
    fn ptr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

fn bytes(v: &Value) -> Vec<u8> {
    v.as_str().and_then(|s| B64.decode(s).ok()).unwrap_or_default()
}

fn b64(p: *const u8, n: u32) -> String {
    if p.is_null() || n == 0 {
        return String::new();
    }
    // SAFETY: webauthn.dll hands out `n` readable bytes at `p`.
    B64.encode(unsafe { std::slice::from_raw_parts(p, n as usize) })
}

fn api_version() -> u32 {
    // SAFETY: no arguments; present on every Windows 10 1903+.
    unsafe { WebAuthNGetApiVersionNumber() }
}

fn transports_of(mask: u32) -> Vec<&'static str> {
    [
        (WEBAUTHN_CTAP_TRANSPORT_USB, "usb"),
        (WEBAUTHN_CTAP_TRANSPORT_NFC, "nfc"),
        (WEBAUTHN_CTAP_TRANSPORT_BLE, "ble"),
        (WEBAUTHN_CTAP_TRANSPORT_INTERNAL, "internal"),
        (WEBAUTHN_CTAP_TRANSPORT_HYBRID, "hybrid"),
    ]
    .into_iter()
    .filter(|(bit, _)| mask & bit != 0)
    .map(|(_, name)| name)
    .collect()
}

fn transport_mask(v: &Value) -> u32 {
    v.as_array().into_iter().flatten().fold(0, |m, t| {
        m | match t.as_str() {
            Some("usb") => WEBAUTHN_CTAP_TRANSPORT_USB,
            Some("nfc") => WEBAUTHN_CTAP_TRANSPORT_NFC,
            Some("ble") => WEBAUTHN_CTAP_TRANSPORT_BLE,
            Some("internal") => WEBAUTHN_CTAP_TRANSPORT_INTERNAL,
            Some("hybrid") => WEBAUTHN_CTAP_TRANSPORT_HYBRID,
            _ => 0,
        }
    })
}

fn attachment_of(transport: u32) -> Value {
    match transport {
        0 => Value::Null,
        WEBAUTHN_CTAP_TRANSPORT_INTERNAL => "platform".into(),
        _ => "cross-platform".into(),
    }
}

fn user_verification(v: &Value) -> u32 {
    match v.as_str() {
        Some("required") => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        Some("discouraged") => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
        _ => WEBAUTHN_USER_VERIFICATION_REQUIREMENT_PREFERRED,
    }
}

/// The credential list webauthn.dll takes: the ids stay owned by `ids`.
struct CredList {
    _ids: Vec<Vec<u8>>,
    _items: Vec<WEBAUTHN_CREDENTIAL_EX>,
    _ptrs: Vec<*mut WEBAUTHN_CREDENTIAL_EX>,
    list: WEBAUTHN_CREDENTIAL_LIST,
}

fn cred_list(v: &Value) -> Box<CredList> {
    let entries: Vec<&Value> = v.as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let mut ids: Vec<Vec<u8>> = entries.iter().map(|e| bytes(&e["id"])).collect();
    let mut items: Vec<WEBAUTHN_CREDENTIAL_EX> = entries
        .iter()
        .zip(ids.iter_mut())
        .map(|(e, id)| WEBAUTHN_CREDENTIAL_EX {
            dwVersion: WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION,
            cbId: id.len() as u32,
            pbId: id.as_mut_ptr(),
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            dwTransports: transport_mask(&e["transports"]),
        })
        .collect();
    let mut ptrs: Vec<*mut WEBAUTHN_CREDENTIAL_EX> = items.iter_mut().map(|i| i as *mut _).collect();
    let list = WEBAUTHN_CREDENTIAL_LIST { cCredentials: ptrs.len() as u32, ppCredentials: ptrs.as_mut_ptr() };
    Box::new(CredList { _ids: ids, _items: items, _ptrs: ptrs, list })
}

fn failure(e: windows::core::Error) -> Fail {
    let hr: HRESULT = e.code();
    // SAFETY: returns a static string for any HRESULT.
    let name = unsafe { WebAuthNGetErrorName(hr).to_string() }.unwrap_or_default();
    let name: &'static str = match name.as_str() {
        "InvalidStateError" => "InvalidStateError",
        "ConstraintError" => "ConstraintError",
        "NotSupportedError" => "NotSupportedError",
        "UnknownError" => "UnknownError",
        _ => "NotAllowedError",
    };
    let message = match name {
        "NotAllowedError" => "The operation either timed out or was not allowed.".to_string(),
        "InvalidStateError" => "The user attempted to register an authenticator that contains one of the credentials already registered with the relying party.".to_string(),
        _ => e.message(),
    };
    Fail::new(name, message).os(hr.0)
}

fn cancellation(token: u32) -> Option<GUID> {
    // SAFETY: no arguments.
    let id = unsafe { WebAuthNGetCancellationId() }.ok()?;
    CANCEL.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashMap::new).insert(token, id);
    Some(id)
}

fn finished(token: u32) {
    if let Some(m) = CANCEL.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        m.remove(&token);
    }
}

pub fn cancel(token: u32) {
    let id = CANCEL.lock().unwrap_or_else(|p| p.into_inner()).as_ref().and_then(|m| m.get(&token).copied());
    if let Some(id) = id {
        // SAFETY: `id` came from WebAuthNGetCancellationId.
        let _ = unsafe { WebAuthNCancelCurrentOperation(&id) };
    }
}

pub fn available() -> bool {
    // SAFETY: no arguments.
    unsafe { WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable() }.is_ok_and(|b| b.as_bool())
}

fn spawn(token: u32, work: impl FnOnce() -> Result<String, Fail> + Send + 'static) -> Result<u32, Fail> {
    let started = std::thread::Builder::new().name("vn-webauthn".into()).spawn(move || {
        let r = work();
        finished(token);
        crate::settle(token, r);
    });
    match started {
        Ok(_) => Ok(token),
        Err(e) => Err(Fail::new("UnknownError", format!("could not start the request: {e}"))),
    }
}

/// `navigator.credentials.create`: the token settled with the attestation.
pub fn start_create(hwnd: isize, req: Value) -> Result<u32, Fail> {
    let token = crate::next_token();
    spawn(token, move || create(hwnd, token, &req))
}

/// `navigator.credentials.get`: the token settled with the assertion.
pub fn start_get(hwnd: isize, req: Value) -> Result<u32, Fail> {
    let token = crate::next_token();
    spawn(token, move || get(hwnd, token, &req))
}

fn create(hwnd: isize, token: u32, req: &Value) -> Result<String, Fail> {
    let s = |k: &str| req[k].as_str().unwrap_or("");
    let (rp_id, rp_name) = (Wide::new(s("rpId")), Wide::new(s("rpName")));
    let (user_name, user_display) = (Wide::new(s("userName")), Wide::new(s("userDisplayName")));
    let mut user_id = bytes(&req["userId"]);
    let mut client_data = s("clientDataJSON").as_bytes().to_vec();
    let rp = WEBAUTHN_RP_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
        pwszId: rp_id.ptr(),
        pwszName: rp_name.ptr(),
        pwszIcon: PCWSTR::null(),
    };
    let user = WEBAUTHN_USER_ENTITY_INFORMATION {
        dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
        cbId: user_id.len() as u32,
        pbId: user_id.as_mut_ptr(),
        pwszName: user_name.ptr(),
        pwszIcon: PCWSTR::null(),
        pwszDisplayName: user_display.ptr(),
    };
    let mut params: Vec<WEBAUTHN_COSE_CREDENTIAL_PARAMETER> = req["algs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_i64())
        .map(|alg| WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            lAlg: alg as i32,
        })
        .collect();
    let cose = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS { cCredentialParameters: params.len() as u32, pCredentialParameters: params.as_mut_ptr() };
    let client = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data.len() as u32,
        pbClientDataJSON: client_data.as_mut_ptr(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };
    let mut exclude = cred_list(&req["exclude"]);
    let mut cancel_id = cancellation(token).unwrap_or_default();
    let api = api_version();
    let resident = s("residentKey");
    let mut opts = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
        dwVersion: match api {
            0..=2 => 3,
            3 => 4,
            4 | 5 => 5,
            6 => 6,
            _ => WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_CURRENT_VERSION,
        },
        dwTimeoutMilliseconds: req["timeout"].as_u64().unwrap_or(300_000) as u32,
        dwAuthenticatorAttachment: match s("attachment") {
            "platform" => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_PLATFORM,
            "cross-platform" => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            _ => WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
        },
        bRequireResidentKey: (resident == "required").into(),
        dwUserVerificationRequirement: user_verification(&req["userVerification"]),
        dwAttestationConveyancePreference: match s("attestation") {
            "direct" | "enterprise" => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT,
            "indirect" => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_INDIRECT,
            _ => WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        },
        pCancellationId: &mut cancel_id,
        pExcludeCredentialList: &mut exclude.list,
        ..Default::default()
    };
    if opts.dwVersion >= 4 {
        opts.bPreferResidentKey = (resident == "preferred").into();
    }
    // SAFETY: every pointer above refers to locals that outlive this blocking call.
    let att = unsafe { WebAuthNAuthenticatorMakeCredential(HWND(hwnd as *mut c_void), &rp, &user, &cose, &client, Some(&opts)) }.map_err(failure)?;
    // SAFETY: `att` is the attestation webauthn.dll allocated, read per its version, then freed.
    unsafe {
        let a = &*att;
        let out = (a.dwVersion >= 3).then(|| {
            let auth = std::slice::from_raw_parts(a.pbAuthenticatorData, a.cbAuthenticatorData as usize);
            let key = cose_public_key(auth);
            json!({
                "id": b64(a.pbCredentialId, a.cbCredentialId),
                "attestationObject": b64(a.pbAttestationObject, a.cbAttestationObject),
                "authenticatorData": B64.encode(auth),
                "transports": transports_of(a.dwUsedTransport),
                "attachment": attachment_of(a.dwUsedTransport),
                "publicKey": key.as_ref().map(|(_, der)| B64.encode(der)),
                "publicKeyAlgorithm": key.as_ref().map(|(alg, _)| *alg).or_else(|| cose_alg(auth)),
                "residentKey": (a.dwVersion >= 4).then(|| a.bResidentKey.as_bool()),
            })
        });
        WebAuthNFreeCredentialAttestation(Some(att));
        out.map(|v| v.to_string()).ok_or_else(|| Fail::new("NotSupportedError", "This version of Windows returns no attestation object."))
    }
}

fn get(hwnd: isize, token: u32, req: &Value) -> Result<String, Fail> {
    let rp_id = Wide::new(req["rpId"].as_str().unwrap_or(""));
    let mut client_data = req["clientDataJSON"].as_str().unwrap_or("").as_bytes().to_vec();
    let client = WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: client_data.len() as u32,
        pbClientDataJSON: client_data.as_mut_ptr(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    };
    let mut allow = cred_list(&req["allow"]);
    let mut cancel_id = cancellation(token).unwrap_or_default();
    let opts = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
        dwVersion: match api_version() {
            0..=2 => 4,
            3 => 5,
            4..=6 => 6,
            _ => WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_CURRENT_VERSION,
        },
        dwTimeoutMilliseconds: req["timeout"].as_u64().unwrap_or(300_000) as u32,
        dwUserVerificationRequirement: user_verification(&req["userVerification"]),
        pCancellationId: &mut cancel_id,
        pAllowCredentialList: &mut allow.list,
        ..Default::default()
    };
    // SAFETY: every pointer above refers to locals that outlive this blocking call.
    let a = unsafe { WebAuthNAuthenticatorGetAssertion(HWND(hwnd as *mut c_void), rp_id.ptr(), &client, Some(&opts)) }.map_err(failure)?;
    // SAFETY: `a` is the assertion webauthn.dll allocated, read per its version, then freed.
    unsafe {
        let r = &*a;
        let transport = if r.dwVersion >= 4 { r.dwUsedTransport } else { 0 };
        let out = json!({
            "id": b64(r.Credential.pbId, r.Credential.cbId),
            "authenticatorData": b64(r.pbAuthenticatorData, r.cbAuthenticatorData),
            "signature": b64(r.pbSignature, r.cbSignature),
            "userHandle": (r.cbUserId > 0).then(|| b64(r.pbUserId, r.cbUserId)),
            "attachment": attachment_of(transport),
        });
        WebAuthNFreeAssertion(a);
        Ok(out.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_round_trip() {
        let m = transport_mask(&json!(["usb", "hybrid", "bogus"]));
        assert_eq!(transports_of(m), vec!["usb", "hybrid"]);
        assert_eq!(attachment_of(WEBAUTHN_CTAP_TRANSPORT_INTERNAL), json!("platform"));
        assert!(api_version() >= 1);
    }
}
