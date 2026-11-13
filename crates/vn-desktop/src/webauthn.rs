//! Passkeys (`navigator.credentials` with `publicKey`) through webauthn.dll: Windows Hello, security
//! keys and phones over hybrid. runtime.ts builds the client data; binary fields cross as base64url.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde_json::{json, Value};
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Networking::WindowsWebServices::*;

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

fn spawn(token: u32, work: impl FnOnce() -> Result<String, Fail> + Send + 'static) -> String {
    let started = std::thread::Builder::new().name("vn-webauthn".into()).spawn(move || {
        let r = work();
        finished(token);
        crate::settle(token, r);
    });
    match started {
        Ok(_) => json!({ "token": token }).to_string(),
        Err(e) => Fail::new("UnknownError", format!("could not start the request: {e}")).to_sync_json(),
    }
}

/// `navigator.credentials.create`: `{ token }`, settled with the attestation.
pub fn start_create(hwnd: isize, req: Value) -> String {
    let token = crate::next_token();
    spawn(token, move || create(hwnd, token, &req))
}

/// `navigator.credentials.get`: `{ token }`, settled with the assertion.
pub fn start_get(hwnd: isize, req: Value) -> String {
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

/// A minimal CBOR reader for the COSE key in authenticator data.
struct Cbor<'a> {
    b: &'a [u8],
    at: usize,
}

enum Item<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    Other,
}

impl<'a> Cbor<'a> {
    fn head(&mut self) -> Option<(u8, u64)> {
        let first = *self.b.get(self.at)?;
        self.at += 1;
        let (major, info) = (first >> 5, first & 0x1f);
        let n = match info {
            0..=23 => info as u64,
            24..=27 => {
                let len = 1usize << (info - 24);
                let raw = self.b.get(self.at..self.at + len)?;
                self.at += len;
                raw.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64)
            }
            _ => return None,
        };
        Some((major, n))
    }

    fn item(&mut self) -> Option<Item<'a>> {
        let (major, n) = self.head()?;
        Some(match major {
            0 => Item::Int(i64::try_from(n).ok()?),
            1 => Item::Int(-1 - i64::try_from(n).ok()?),
            2 | 3 => {
                let end = self.at.checked_add(usize::try_from(n).ok()?)?;
                let raw = self.b.get(self.at..end)?;
                self.at = end;
                if major == 2 {
                    Item::Bytes(raw)
                } else {
                    Item::Other
                }
            }
            4 => {
                for _ in 0..n {
                    self.item()?;
                }
                Item::Other
            }
            5 => {
                for _ in 0..n.checked_mul(2)? {
                    self.item()?;
                }
                Item::Other
            }
            6 => {
                self.item()?;
                Item::Other
            }
            _ => Item::Other,
        })
    }
}

fn cose_map(auth: &[u8]) -> Option<HashMap<i64, Item<'_>>> {
    if auth.len() < 55 || auth[32] & 0x40 == 0 {
        return None;
    }
    let id_len = u16::from_be_bytes([auth[53], auth[54]]) as usize;
    let mut r = Cbor { b: auth, at: 55 + id_len };
    let (major, n) = r.head()?;
    if major != 5 {
        return None;
    }
    let mut m = HashMap::new();
    for _ in 0..n {
        let Item::Int(k) = r.item()? else { return None };
        m.insert(k, r.item()?);
    }
    Some(m)
}

fn cose_alg(auth: &[u8]) -> Option<i64> {
    match cose_map(auth)?.get(&3)? {
        Item::Int(a) => Some(*a),
        _ => None,
    }
}

fn der_len(n: usize) -> Vec<u8> {
    match n {
        0..=127 => vec![n as u8],
        128..=255 => vec![0x81, n as u8],
        _ => vec![0x82, (n >> 8) as u8, n as u8],
    }
}

fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(der_len(body.len()));
    out.extend_from_slice(body);
    out
}

fn der_uint(v: &[u8]) -> Vec<u8> {
    let v = &v[v.iter().position(|&b| b != 0).unwrap_or(v.len().saturating_sub(1))..];
    let mut body = Vec::with_capacity(v.len() + 1);
    if v.first().is_some_and(|&b| b & 0x80 != 0) {
        body.push(0);
    }
    body.extend_from_slice(v);
    der(0x02, &body)
}

/// `(alg, SubjectPublicKeyInfo DER)` for ES256, EdDSA (Ed25519) and RS256 keys, as
/// `getPublicKey()` returns them.
fn cose_public_key(auth: &[u8]) -> Option<(i64, Vec<u8>)> {
    let m = cose_map(auth)?;
    let int = |k: i64| match m.get(&k) {
        Some(Item::Int(v)) => Some(*v),
        _ => None,
    };
    let raw = |k: i64| match m.get(&k) {
        Some(Item::Bytes(b)) => Some(*b),
        _ => None,
    };
    let alg = int(3)?;
    let spki = match (int(1)?, int(-1)) {
        (2, Some(1)) => {
            let (x, y) = (raw(-2)?, raw(-3)?);
            if x.len() != 32 || y.len() != 32 {
                return None;
            }
            let mut point = vec![0x04];
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            let mut out = hex("3059301306072a8648ce3d020106082a8648ce3d030107034200");
            out.extend(point);
            out
        }
        (1, Some(6)) => {
            let x = raw(-2)?;
            if x.len() != 32 {
                return None;
            }
            let mut out = hex("302a300506032b6570032100");
            out.extend_from_slice(x);
            out
        }
        (3, _) => {
            let (n, e) = (raw(-1)?, raw(-2)?);
            let mut key = der_uint(n);
            key.extend(der_uint(e));
            let mut bits = vec![0];
            bits.extend(der(0x30, &key));
            let mut body = hex("300d06092a864886f70d0101010500");
            body.extend(der(0x03, &bits));
            der(0x30, &body)
        }
        _ => return None,
    };
    Some((alg, spki))
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_with_key(key: &[u8]) -> Vec<u8> {
        let mut a = vec![0u8; 32];
        a.push(0x45);
        a.extend([0, 0, 0, 1]);
        a.extend([0u8; 16]);
        a.extend([0, 2, 0xAA, 0xBB]);
        a.extend_from_slice(key);
        a
    }

    #[test]
    fn an_es256_key_becomes_its_spki() {
        let mut key = vec![0xA5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        key.extend([1u8; 32]);
        key.extend([0x22, 0x58, 0x20]);
        key.extend([2u8; 32]);
        let (alg, spki) = cose_public_key(&auth_with_key(&key)).unwrap();
        assert_eq!(alg, -7);
        assert_eq!(spki.len(), 91);
        assert_eq!(&spki[..27], &hex("3059301306072a8648ce3d020106082a8648ce3d03010703420004")[..]);
        assert_eq!(&spki[27..59], &[1u8; 32]);
    }

    #[test]
    fn an_rs256_key_becomes_its_spki() {
        let mut key = vec![0xA4, 0x01, 0x03, 0x03, 0x39, 0x01, 0x00, 0x20, 0x59, 0x01, 0x00];
        key.extend([0x80u8; 256]);
        key.extend([0x21, 0x43, 0x01, 0x00, 0x01]);
        let (alg, spki) = cose_public_key(&auth_with_key(&key)).unwrap();
        assert_eq!(alg, -257);
        assert_eq!(&spki[..4], &[0x30, 0x82, 0x01, 0x22]);
        assert_eq!(spki.len(), 294);
        assert_eq!(&spki[spki.len() - 5..], &[0x02, 0x03, 0x01, 0x00, 0x01]);
    }

    #[test]
    fn no_attested_data_is_no_key() {
        let mut a = auth_with_key(&[0xA0]);
        a[32] = 0x05;
        assert!(cose_public_key(&a).is_none());
        assert!(cose_public_key(&[0u8; 10]).is_none());
    }

    #[test]
    fn transports_round_trip() {
        let m = transport_mask(&json!(["usb", "hybrid", "bogus"]));
        assert_eq!(transports_of(m), vec!["usb", "hybrid"]);
        assert_eq!(attachment_of(WEBAUTHN_CTAP_TRANSPORT_INTERNAL), json!("platform"));
        assert!(api_version() >= 1);
    }
}
