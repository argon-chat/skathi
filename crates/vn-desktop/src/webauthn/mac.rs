//! AuthenticationServices: passkeys (iCloud Keychain, phones over hybrid) and security keys, offered
//! together unless the request names an attachment. The system writes and signs its own client data.
//!
//! Apple serves a relying party only to an app associated with it: the app must be signed with a
//! provisioning profile granting the Associated Domains entitlement `webcredentials:<rpId>`, and the
//! domain must list the app in its apple-app-site-association file. Without any `webcredentials` entry a
//! request is refused before the system is asked.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{NSApplication, NSWindow};
use objc2_authentication_services::{
    ASAuthorization, ASAuthorizationController, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding, ASAuthorizationCredential, ASAuthorizationError,
    ASAuthorizationPlatformPublicKeyCredentialAssertion, ASAuthorizationPlatformPublicKeyCredentialDescriptor,
    ASAuthorizationPlatformPublicKeyCredentialProvider, ASAuthorizationPlatformPublicKeyCredentialRegistration,
    ASAuthorizationPublicKeyCredentialAssertion, ASAuthorizationPublicKeyCredentialAssertionRequest,
    ASAuthorizationPublicKeyCredentialAttachment, ASAuthorizationPublicKeyCredentialParameters,
    ASAuthorizationPublicKeyCredentialRegistration, ASAuthorizationPublicKeyCredentialRegistrationRequest, ASAuthorizationRequest,
    ASAuthorizationSecurityKeyPublicKeyCredentialAssertion, ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor,
    ASAuthorizationSecurityKeyPublicKeyCredentialProvider, ASAuthorizationSecurityKeyPublicKeyCredentialRegistration,
};
use objc2_foundation::{NSArray, NSData, NSError, NSObjectProtocol, NSString};
use serde_json::{json, Value};

use super::{auth_data_of, cose_alg, cose_public_key};
use crate::Fail;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecTaskCreateFromSelf(allocator: *const c_void) -> *mut c_void;
    fn SecTaskCopyValueForEntitlement(task: *mut c_void, entitlement: *const c_void, error: *mut *mut c_void) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// The domains of the app's `webcredentials:` associated domains.
fn associated_domains() -> Vec<String> {
    let key = NSString::from_str("com.apple.developer.associated-domains");
    // SAFETY: a null allocator is the default; the task is released here and the copied value, a
    // CFArray (toll-free bridged with NSArray) or null, is owned by the `Retained`.
    let value = unsafe {
        let task = SecTaskCreateFromSelf(std::ptr::null());
        if task.is_null() {
            return Vec::new();
        }
        let v = SecTaskCopyValueForEntitlement(task, Retained::as_ptr(&key).cast(), std::ptr::null_mut());
        CFRelease(task);
        Retained::<AnyObject>::from_raw(v.cast())
    };
    let Some(list) = value.as_deref().and_then(|v| v.downcast_ref::<NSArray>()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()))
        .filter_map(|s| s.strip_prefix("webcredentials:").map(|d| d.split('?').next().unwrap_or_default().to_ascii_lowercase()))
        .collect()
}

fn association_needed(rp_id: &str) -> String {
    format!(
        "This app is not associated with '{rp_id}'. Passkeys on macOS need the app signed with a provisioning profile \
         that grants the Associated Domains entitlement 'webcredentials:{rp_id}', and \
         https://{rp_id}/.well-known/apple-app-site-association listing the app under 'webcredentials'."
    )
}

pub fn available() -> bool {
    !associated_domains().is_empty()
}

struct Ivars {
    token: u32,
    owner: isize,
    rp_id: String,
}

struct Live {
    controller: Retained<ASAuthorizationController>,
    delegate: Retained<Delegate>,
}

thread_local! {
    /// Requests in flight; the controller holds its delegate weakly.
    static LIVE: RefCell<HashMap<u32, Live>> = RefCell::new(HashMap::new());
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Delegate does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VueNativePasskeyDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl ASAuthorizationControllerDelegate for Delegate {
        #[unsafe(method(authorizationController:didCompleteWithAuthorization:))]
        fn did_complete(&self, _controller: &ASAuthorizationController, authorization: &ASAuthorization) {
            // SAFETY: the credential of a finished authorization.
            let credential = unsafe { authorization.credential() };
            self.finish(read(&credential).ok_or_else(|| Fail::new("NotAllowedError", "The system returned no public key credential.")));
        }

        #[unsafe(method(authorizationController:didCompleteWithError:))]
        fn did_fail(&self, _controller: &ASAuthorizationController, error: &NSError) {
            self.finish(Err(failure(error, &self.ivars().rp_id)));
        }
    }

    unsafe impl ASAuthorizationControllerPresentationContextProviding for Delegate {
        #[unsafe(method_id(presentationAnchorForAuthorizationController:))]
        fn anchor(&self, _controller: &ASAuthorizationController) -> Retained<NSObject> {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            let window = crate::dialog::window_of(mtm, self.ivars().owner)
                .or_else(|| app.keyWindow())
                .or_else(|| app.mainWindow())
                .or_else(|| app.windows().firstObject())
                // SAFETY: a plain, never shown window, as a last resort for the system's sheet.
                .unwrap_or_else(|| unsafe { NSWindow::new(mtm) });
            window.into_super().into_super()
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, ivars: Ivars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, result: Result<Value, Fail>) {
        let token = self.ivars().token;
        let Some(live) = LIVE.with(|l| l.borrow_mut().remove(&token)) else { return };
        crate::settle(token, result.map(|v| v.to_string()));
        // The controller is mid-callback: the run loop's autorelease pool lets go of both afterwards.
        let _ = Retained::autorelease_ptr(live.controller);
        let _ = Retained::autorelease_ptr(live.delegate);
    }
}

fn failure(e: &NSError, rp_id: &str) -> Fail {
    let code = e.code();
    let (name, message) = if code == ASAuthorizationError::MatchedExcludedCredential.0 {
        (
            "InvalidStateError",
            "The user attempted to register an authenticator that contains one of the credentials already registered with the relying party.".to_string(),
        )
    } else if code == ASAuthorizationError::Failed.0 || code == ASAuthorizationError::NotHandled.0 {
        ("NotAllowedError", format!("{} {}", e.localizedDescription(), association_needed(rp_id)))
    } else {
        ("NotAllowedError", "The operation either timed out or was not allowed.".to_string())
    };
    Fail::new(name, message).os(code as i32)
}

fn b64(d: &NSData) -> String {
    B64.encode(d.to_vec())
}

fn registration<T: ASAuthorizationPublicKeyCredentialRegistration + Message>(r: &T, attachment: &str, transports: &[&str], resident: Option<bool>) -> Value {
    // SAFETY: getters of a finished registration.
    let (id, client_data, attestation) = unsafe { (r.credentialID(), r.rawClientDataJSON(), r.rawAttestationObject()) };
    let attestation = attestation.map(|d| d.to_vec()).unwrap_or_default();
    let auth = auth_data_of(&attestation).unwrap_or_default();
    let key = cose_public_key(auth);
    json!({
        "id": b64(&id),
        "clientDataJSON": b64(&client_data),
        "attestationObject": B64.encode(&attestation),
        "authenticatorData": B64.encode(auth),
        "transports": transports,
        "attachment": attachment,
        "publicKey": key.as_ref().map(|(_, der)| B64.encode(der)),
        "publicKeyAlgorithm": key.as_ref().map(|(alg, _)| *alg).or_else(|| cose_alg(auth)),
        "residentKey": resident,
    })
}

fn assertion<T: ASAuthorizationPublicKeyCredentialAssertion + Message>(a: &T, attachment: &str) -> Value {
    // SAFETY: getters of a finished assertion.
    let (id, client_data, auth, signature, user) =
        unsafe { (a.credentialID(), a.rawClientDataJSON(), a.rawAuthenticatorData(), a.signature(), a.userID()) };
    json!({
        "id": b64(&id),
        "clientDataJSON": b64(&client_data),
        "authenticatorData": b64(&auth),
        "signature": b64(&signature),
        "userHandle": (!user.is_empty()).then(|| b64(&user)),
        "attachment": attachment,
    })
}

/// A platform credential's attachment: `cross-platform` when a phone answered over hybrid (macOS 13.5+).
fn platform_attachment(obj: &NSObject, read: impl FnOnce() -> ASAuthorizationPublicKeyCredentialAttachment) -> &'static str {
    if obj.respondsToSelector(sel!(attachment)) && read() == ASAuthorizationPublicKeyCredentialAttachment::CrossPlatform {
        "cross-platform"
    } else {
        "platform"
    }
}

fn read(credential: &ProtocolObject<dyn ASAuthorizationCredential>) -> Option<Value> {
    let any: &AnyObject = credential.as_ref();
    if let Some(r) = any.downcast_ref::<ASAuthorizationPlatformPublicKeyCredentialRegistration>() {
        // SAFETY: only called when the object answers `attachment`.
        let attachment = platform_attachment(r, || unsafe { r.attachment() });
        return Some(registration(r, attachment, &["hybrid", "internal"], Some(true)));
    }
    if let Some(r) = any.downcast_ref::<ASAuthorizationSecurityKeyPublicKeyCredentialRegistration>() {
        return Some(registration(r, "cross-platform", &["usb"], None));
    }
    if let Some(a) = any.downcast_ref::<ASAuthorizationPlatformPublicKeyCredentialAssertion>() {
        // SAFETY: as above.
        let attachment = platform_attachment(a, || unsafe { a.attachment() });
        return Some(assertion(a, attachment));
    }
    if let Some(a) = any.downcast_ref::<ASAuthorizationSecurityKeyPublicKeyCredentialAssertion>() {
        return Some(assertion(a, "cross-platform"));
    }
    None
}

fn bytes(v: &Value) -> Vec<u8> {
    v.as_str().and_then(|s| B64.decode(s).ok()).unwrap_or_default()
}

fn user_verification(v: &Value) -> Retained<NSString> {
    NSString::from_str(match v.as_str() {
        Some(s @ ("required" | "discouraged")) => s,
        _ => "preferred",
    })
}

fn platform_descriptors(v: &Value) -> Retained<NSArray<ASAuthorizationPlatformPublicKeyCredentialDescriptor>> {
    let list: Vec<_> = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let id = NSData::with_bytes(&bytes(&d["id"]));
            // SAFETY: a fresh descriptor from a credential ID.
            unsafe { ASAuthorizationPlatformPublicKeyCredentialDescriptor::initWithCredentialID(ASAuthorizationPlatformPublicKeyCredentialDescriptor::alloc(), &id) }
        })
        .collect();
    NSArray::from_retained_slice(&list)
}

fn security_key_descriptors(v: &Value) -> Retained<NSArray<ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor>> {
    let list: Vec<_> = v
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let id = NSData::with_bytes(&bytes(&d["id"]));
            let named: Vec<&str> = d["transports"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).filter(|t| matches!(*t, "usb" | "nfc" | "ble")).collect();
            let transports: Vec<_> = (if named.is_empty() { vec!["usb", "nfc", "ble"] } else { named }).into_iter().map(NSString::from_str).collect();
            // SAFETY: a fresh descriptor from a credential ID and transport names.
            unsafe {
                ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor::initWithCredentialID_transports(
                    ASAuthorizationSecurityKeyPublicKeyCredentialDescriptor::alloc(),
                    &id,
                    &NSArray::from_retained_slice(&transports),
                )
            }
        })
        .collect();
    NSArray::from_retained_slice(&list)
}

fn registration_requests(req: &Value) -> Vec<Retained<ASAuthorizationRequest>> {
    let s = |k: &str| req[k].as_str().unwrap_or("");
    let rp = NSString::from_str(s("rpId"));
    let challenge = NSData::with_bytes(&bytes(&req["challenge"]));
    let user_id = NSData::with_bytes(&bytes(&req["userId"]));
    let (name, display) = (NSString::from_str(s("userName")), NSString::from_str(s("userDisplayName")));
    let uv = user_verification(&req["userVerification"]);
    let attestation = NSString::from_str(match s("attestation") {
        a @ ("direct" | "indirect" | "enterprise") => a,
        _ => "none",
    });
    let mut out = Vec::new();
    // SAFETY: requests built from fresh providers and owned values, before any is performed.
    unsafe {
        if s("attachment") != "cross-platform" {
            let provider = ASAuthorizationPlatformPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
                ASAuthorizationPlatformPublicKeyCredentialProvider::alloc(),
                &rp,
            );
            let r = provider.createCredentialRegistrationRequestWithChallenge_name_userID(&challenge, &name, &user_id);
            r.setDisplayName(Some(&*display));
            r.setUserVerificationPreference(&uv);
            r.setAttestationPreference(&attestation);
            let exclude = platform_descriptors(&req["exclude"]);
            if !exclude.is_empty() && r.respondsToSelector(sel!(setExcludedCredentials:)) {
                let _: () = msg_send![&*r, setExcludedCredentials: &*exclude];
            }
            out.push(r.into_super());
        }
        if s("attachment") != "platform" {
            let provider = ASAuthorizationSecurityKeyPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
                ASAuthorizationSecurityKeyPublicKeyCredentialProvider::alloc(),
                &rp,
            );
            let r = provider.createCredentialRegistrationRequestWithChallenge_displayName_name_userID(&challenge, &display, &name, &user_id);
            let params: Vec<_> = req["algs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_i64())
                .map(|alg| ASAuthorizationPublicKeyCredentialParameters::initWithAlgorithm(ASAuthorizationPublicKeyCredentialParameters::alloc(), alg as isize))
                .collect();
            r.setCredentialParameters(&NSArray::from_retained_slice(&params));
            r.setExcludedCredentials(&security_key_descriptors(&req["exclude"]));
            r.setResidentKeyPreference(&NSString::from_str(match s("residentKey") {
                k @ ("required" | "preferred") => k,
                _ => "discouraged",
            }));
            r.setUserVerificationPreference(&uv);
            r.setAttestationPreference(&attestation);
            out.push(r.into_super());
        }
    }
    out
}

fn assertion_requests(req: &Value) -> Vec<Retained<ASAuthorizationRequest>> {
    let rp = NSString::from_str(req["rpId"].as_str().unwrap_or(""));
    let challenge = NSData::with_bytes(&bytes(&req["challenge"]));
    let uv = user_verification(&req["userVerification"]);
    // SAFETY: as in `registration_requests`.
    unsafe {
        let platform = ASAuthorizationPlatformPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
            ASAuthorizationPlatformPublicKeyCredentialProvider::alloc(),
            &rp,
        )
        .createCredentialAssertionRequestWithChallenge(&challenge);
        platform.setAllowedCredentials(&platform_descriptors(&req["allow"]));
        platform.setUserVerificationPreference(&uv);
        let key = ASAuthorizationSecurityKeyPublicKeyCredentialProvider::initWithRelyingPartyIdentifier(
            ASAuthorizationSecurityKeyPublicKeyCredentialProvider::alloc(),
            &rp,
        )
        .createCredentialAssertionRequestWithChallenge(&challenge);
        key.setAllowedCredentials(&security_key_descriptors(&req["allow"]));
        key.setUserVerificationPreference(&uv);
        vec![platform.into_super(), key.into_super()]
    }
}

fn start(owner: isize, req: Value, create: bool) -> Result<u32, Fail> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(Fail::new("NotAllowedError", "Passkey requests can only start on the main thread."));
    };
    let rp_id = req["rpId"].as_str().unwrap_or("").to_string();
    if associated_domains().is_empty() {
        return Err(Fail::new("NotAllowedError", association_needed(&rp_id)));
    }
    let requests = if create { registration_requests(&req) } else { assertion_requests(&req) };
    let token = crate::next_token();
    let delegate = Delegate::new(mtm, Ivars { token, owner, rp_id });
    // SAFETY: the controller and its delegate stay in `LIVE` until the delegate is told the outcome.
    let controller = unsafe {
        let c = ASAuthorizationController::initWithAuthorizationRequests(ASAuthorizationController::alloc(), &NSArray::from_retained_slice(&requests));
        c.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        c.setPresentationContextProvider(Some(ProtocolObject::from_ref(&*delegate)));
        c
    };
    LIVE.with(|l| l.borrow_mut().insert(token, Live { controller: controller.clone(), delegate }));
    // SAFETY: on the main thread, with the delegate set.
    unsafe { controller.performRequests() };
    let timeout = Duration::from_millis(req["timeout"].as_u64().unwrap_or(300_000));
    let _ = DispatchQueue::main().after(DispatchTime::try_from(timeout).unwrap_or(DispatchTime::FOREVER), move || cancel(token));
    Ok(token)
}

/// `navigator.credentials.create`: the token settled with the attestation.
pub fn start_create(owner: isize, req: Value) -> Result<u32, Fail> {
    start(owner, req, true)
}

/// `navigator.credentials.get`: the token settled with the assertion.
pub fn start_get(owner: isize, req: Value) -> Result<u32, Fail> {
    start(owner, req, false)
}

/// Main thread: ends `token`'s request, which then settles as not allowed.
pub fn cancel(token: u32) {
    let controller = LIVE.with(|l| l.borrow().get(&token).map(|l| l.controller.clone()));
    if let Some(c) = controller {
        // SAFETY: a controller this thread performed.
        unsafe { c.cancel() };
    }
}
