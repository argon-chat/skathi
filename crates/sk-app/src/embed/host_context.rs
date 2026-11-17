//! Bootstrap services and direct, thread-confined JSC access for an embedding application.

use std::path::PathBuf;
use std::time::SystemTime;

/// Cookie attributes accepted by the host's HTTP cookie jar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
    Unspecified,
}

/// A host-provisioned cookie. `url` is its origin; normal HTTP cookie rules apply.
#[derive(Clone, Debug)]
pub struct HostCookie {
    pub url: String,
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    /// `None` makes this a session cookie.
    pub expires: Option<SystemTime>,
}

/// Services available after storage is installed and before any application JS runs.
/// Cookie and secret operations need the `storages` feature; secrets are sealed by the OS provider, no plaintext fallback.
pub struct HostContext<'a> {
    runtime: Option<&'a sk_runtime::JsRuntime>,
    js: Option<&'a crate::js::Context>,
    storage_dir: Option<PathBuf>,
}

impl<'a> HostContext<'a> {
    pub(super) fn installed(runtime: Option<&'a sk_runtime::JsRuntime>) -> Self {
        Self {
            runtime,
            js: runtime.map(sk_runtime::JsRuntime::ctx),
            #[cfg(feature = "storages")]
            storage_dir: sk_storage::cookies::folder(),
            #[cfg(not(feature = "storages"))]
            storage_dir: None,
        }
    }

    /// Direct JavaScriptCore API; UI/VM thread only.
    pub fn js(&self) -> &'a crate::js::Context {
        self.js
            .expect("HostContext::js is only available inside Builder::setup")
    }

    /// Installs a native namespace before app code, built with `js.object`, `js.set` and `sk_app::js::function`.
    pub fn expose(&self, name: &str, value: crate::js::Value) -> crate::js::Result<()> {
        if name.is_empty()
            || name.to_ascii_lowercase().starts_with("__sk")
            || !self.js().global_get(name)?.is_undefined()
        {
            return Err(crate::js::Error::BadArg);
        }
        self.js().global_set(name, value)
    }

    /// A bounded worker-to-VM queue. Capture owned Rust data, never JSC values.
    pub fn dispatcher(&self) -> crate::js::JsDispatcher {
        crate::js::JsDispatcher::install(self.js())
    }

    /// The running app's [`super::Handle`], for UI that outlives setup (a tray icon). On macOS it
    /// is the only way to one: `run_on_main_thread` returns no handle.
    pub fn handle(&self) -> super::Handle {
        super::Handle { _private: () }
    }

    /// The engine's runtime, for an extension built against the same `sk-runtime` that registers
    /// `__sk_*` host fns, which [`Self::expose`] refuses on purpose. Prefer `expose`.
    pub fn runtime(&self) -> &'a sk_runtime::JsRuntime {
        self.runtime
            .expect("HostContext::runtime is only available inside Builder::setup")
    }

    /// The surface `<sk-surface name="…">` elements show: present frames to it from any thread.
    pub fn surface(&self, name: &str) -> super::SurfaceHandle {
        sk_tree::surface::surface(name)
    }

    /// Called on the VM thread once per frame before the frame's wave drains: deliver what worker
    /// threads queued here, so a JS handler never observes a half-applied patch. Keep it bounded;
    /// it is on the frame's critical path. Runs until shutdown.
    pub fn on_frame(&self, hook: impl Fn() + 'static) {
        crate::js::on_frame(hook);
    }

    /// Called on the VM thread after old-page teardown, before its replacement.
    /// Clear and unprotect page-owned callbacks here. The namespace remains installed.
    pub fn on_reload(&self, hook: impl Fn(&crate::js::Context) + 'static) {
        crate::js::on_reload(hook);
    }

    /// Called once on the VM thread during orderly shutdown. Release protected callbacks here;
    /// stopping workers is the embedder's job.
    pub fn on_shutdown(&self, hook: impl Fn(&crate::js::Context) + 'static) {
        crate::js::on_shutdown(hook);
    }

    pub fn storage_dir(&self) -> Result<PathBuf, String> {
        self.storage_dir.clone().ok_or_else(storage_unavailable)
    }

    /// Provisions a cookie, including HttpOnly cookies, before the first request.
    pub fn set_cookie(&self, cookie: HostCookie) -> Result<(), String> {
        self.storage_dir()?;
        #[cfg(feature = "storages")]
        {
            use sk_runtime::redjs::webapi::cookies as jar;
            jar::set(jar::HostCookie {
                url: cookie.url,
                name: cookie.name,
                value: cookie.value,
                domain: cookie.domain,
                path: cookie.path,
                secure: cookie.secure,
                http_only: cookie.http_only,
                same_site: match cookie.same_site {
                    SameSite::None => jar::SameSite::None,
                    SameSite::Lax => jar::SameSite::Lax,
                    SameSite::Strict => jar::SameSite::Strict,
                    SameSite::Unspecified => jar::SameSite::Unspecified,
                },
                expires: cookie.expires,
            })
        }
        #[cfg(not(feature = "storages"))]
        {
            let _ = cookie;
            Err(storage_unavailable())
        }
    }

    pub fn random_bytes(&self, bytes: &mut [u8]) -> Result<(), String> {
        #[cfg(feature = "storages")]
        {
            sk_storage::crypt::random_bytes(bytes)
        }
        #[cfg(not(feature = "storages"))]
        {
            let _ = bytes;
            Err(storage_unavailable())
        }
    }

    /// Only an absent file returns `Ok(None)`; a corrupt or undecryptable secret is an error.
    pub fn read_secret(&self, name: &str) -> Result<Option<String>, String> {
        validate_secret_name(name)?;
        let folder = self.storage_dir()?;
        #[cfg(feature = "storages")]
        {
            let provider = sk_storage::crypt::provider().ok_or_else(|| {
                "host secrets: no OS encryption provider on this platform".to_string()
            })?;
            let path = folder.join(name);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(format!("host secrets: could not read {name}: {e}")),
            };
            sk_storage::file::unseal(provider, &bytes)
                .map(Some)
                .map_err(|e| format!("host secrets: could not decrypt {name}: {e}"))
        }
        #[cfg(not(feature = "storages"))]
        {
            let _ = folder;
            Err(storage_unavailable())
        }
    }

    pub fn write_secret(&self, name: &str, value: &str) -> Result<(), String> {
        validate_secret_name(name)?;
        let folder = self.storage_dir()?;
        #[cfg(feature = "storages")]
        {
            let provider = sk_storage::crypt::provider().ok_or_else(|| {
                "host secrets: no OS encryption provider on this platform".to_string()
            })?;
            let bytes = sk_storage::file::seal(provider, value)?;
            std::fs::create_dir_all(&folder)
                .map_err(|e| format!("host secrets: could not create storage directory: {e}"))?;
            sk_storage::file::write_atomic(&folder.join(name), &bytes)
                .map_err(|e| format!("host secrets: could not write {name}: {e}"))
        }
        #[cfg(not(feature = "storages"))]
        {
            let _ = (folder, value);
            Err(storage_unavailable())
        }
    }
}

fn storage_unavailable() -> String {
    "host storage services are unavailable: enable sk-app's storages feature and run setup after storage installation".into()
}

fn validate_secret_name(name: &str) -> Result<(), String> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    let reserved = matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty()
        || name.len() > 128
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || reserved
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(
            "host secrets: name must be one ordinary filename (letters, digits, '.', '_' or '-')"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_cannot_escape_the_storage_directory_or_name_windows_devices() {
        for name in [
            "",
            ".",
            "..",
            "../secret",
            "x/y",
            r"x\y",
            "C:secret",
            "secret:stream",
            "con",
            "NUL.sealed",
            "COM1",
            "lpt9.txt",
            "trailing.",
        ] {
            assert!(validate_secret_name(name).is_err(), "{name:?}");
        }
        assert!(validate_secret_name("device-id.sealed").is_ok());
    }

    #[test]
    fn context_without_storage_refuses_credential_operations() {
        let ctx = HostContext {
            runtime: None,
            js: None,
            storage_dir: None,
        };
        assert!(ctx.storage_dir().is_err());
        assert!(ctx.read_secret("test.sealed").is_err());
        assert!(ctx.write_secret("test.sealed", "secret").is_err());
        let cookie = HostCookie {
            url: "https://host-context.test".into(),
            name: "bootstrap".into(),
            value: "value".into(),
            domain: None,
            path: Some("/".into()),
            secure: true,
            http_only: true,
            same_site: SameSite::None,
            expires: None,
        };
        assert!(ctx.set_cookie(cookie).is_err());
        #[cfg(not(feature = "storages"))]
        assert!(ctx.random_bytes(&mut [0; 8]).is_err());
    }

    #[test]
    #[cfg(all(feature = "storages", target_os = "windows"))]
    fn secrets_preserve_the_existing_sealed_format_and_refuse_corruption() {
        let folder = std::env::temp_dir().join(format!(
            "sk-host-secret-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ctx = HostContext {
            runtime: None,
            js: None,
            storage_dir: Some(folder.clone()),
        };
        let name = "device-id.sealed";
        assert_eq!(ctx.read_secret(name).unwrap(), None);
        ctx.write_secret(name, "stable-device-id").unwrap();
        assert_eq!(
            ctx.read_secret(name).unwrap().as_deref(),
            Some("stable-device-id")
        );
        let bytes = std::fs::read(folder.join(name)).unwrap();
        assert!(!bytes.windows(16).any(|b| b == b"stable-device-id"));
        let provider = sk_storage::crypt::provider().unwrap();
        assert_eq!(
            sk_storage::file::unseal(provider, &bytes).unwrap(),
            "stable-device-id"
        );
        let legacy = sk_storage::file::seal(provider, "legacy-device-id").unwrap();
        std::fs::write(folder.join(name), legacy).unwrap();
        assert_eq!(
            ctx.read_secret(name).unwrap().as_deref(),
            Some("legacy-device-id")
        );
        std::fs::write(folder.join(name), b"corrupt").unwrap();
        assert!(ctx.read_secret(name).is_err());
        std::fs::remove_file(folder.join(name)).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    #[cfg(feature = "storages")]
    fn generic_cookie_setter_obeys_origin_and_domain_rules() {
        let ctx = HostContext {
            runtime: None,
            js: None,
            storage_dir: Some(PathBuf::from("unused-cookie-test-folder")),
        };
        let mut cookie = HostCookie {
            url: "https://host-context.test".into(),
            name: "sk-host-bootstrap-test".into(),
            value: "value".into(),
            domain: Some("elsewhere.test".into()),
            path: Some("/".into()),
            secure: true,
            http_only: true,
            same_site: SameSite::None,
            expires: None,
        };
        assert!(ctx.set_cookie(cookie.clone()).is_err());
        cookie.domain = None;
        assert!(ctx.set_cookie(cookie).is_ok());
        sk_runtime::redjs::webapi::cookies::remove(
            "https://host-context.test",
            "sk-host-bootstrap-test",
        );
    }
}
