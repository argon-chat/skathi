//! MSIX owns the COM registration. Only the installed executable declared as
//! the toast server can acquire the class and accept activations after exit.
use super::{activate, reply, Apartment, Error, Notice};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Once, OnceLock},
    time::Duration,
};
use uuid::Uuid;
use windows::{
    core::{h, implement, IUnknown, Interface, Ref, BOOL, GUID, PCWSTR},
    ApplicationModel::Package,
    Data::Xml::Dom::XmlDocument,
    Storage::ApplicationData,
    Win32::{
        Foundation::{CLASS_E_NOAGGREGATION, E_POINTER},
        System::{
            Com::{
                CoRegisterClassObject, IClassFactory, IClassFactory_Impl, CLSCTX_LOCAL_SERVER,
                REGCLS_MULTIPLEUSE,
            },
            WinRT::{RoInitialize, RO_INIT_MULTITHREADED},
        },
        UI::Notifications::{
            INotificationActivationCallback, INotificationActivationCallback_Impl,
            NOTIFICATION_USER_INPUT_DATA,
        },
    },
};

static START: Once = Once::new();
static ROUTES: OnceLock<Option<Routes>> = OnceLock::new();

pub(crate) fn background_launch() -> bool {
    sk_desktop::args::process_toast_activation()
}

pub(crate) fn initialize() {
    START.call_once(|| {
        if sk_permissions::package::app_user_model_id().is_none()
            || crate::embed::profile_overridden()
        {
            let _ = ROUTES.set(None);
            return;
        }
        if std::thread::Builder::new()
            .name("sk-toast-activation".into())
            .spawn(|| {
                match register() {
                    Ok((routes, _apartment)) => {
                        let _ = ROUTES.set(Some(routes));
                        // COM retains the registered factory and dispatches calls in this MTA.
                        loop {
                            std::thread::park();
                        }
                    }
                    Err(error) => {
                        log::debug!("MSIX notification activation unavailable: {error}");
                        let _ = ROUTES.set(None);
                    }
                }
            })
            .is_err()
        {
            let _ = ROUTES.set(None);
        }
    });
}

pub(super) fn routes() -> Option<&'static Routes> {
    initialize();
    ROUTES.wait().as_ref()
}

fn register() -> anyhow::Result<(Routes, Apartment)> {
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
    let apartment = Apartment;
    let package_root = PathBuf::from(
        Package::Current()?
            .InstalledLocation()?
            .Path()?
            .to_os_string(),
    );
    let manifest = XmlDocument::new()?;
    manifest.LoadXml(&fs::read_to_string(package_root.join("AppxManifest.xml"))?.into())?;
    let server = manifest.SelectSingleNode(h!("//*[local-name()='Application' and @Id='App']//*[local-name()='ExeServer' and @Arguments='--sk-toast-activation']"))?;
    let executable = server
        .SelectSingleNode(h!("@Executable"))?
        .InnerText()?
        .to_os_string();
    anyhow::ensure!(
        package_root.join(executable).canonicalize()? == std::env::current_exe()?.canonicalize()?,
        "not this package's toast executable"
    );
    let server_id = server
        .SelectSingleNode(h!("*[local-name()='Class']/@Id"))?
        .InnerText()?
        .to_string();
    let toast_id = manifest.SelectSingleNode(h!("//*[local-name()='Application' and @Id='App']//*[local-name()='ToastNotificationActivation']/@ToastActivatorCLSID"))?.InnerText()?.to_string();
    anyhow::ensure!(
        toast_id.eq_ignore_ascii_case(&server_id),
        "toast registration and COM class differ"
    );
    let clsid = GUID::try_from(server_id.as_str())?;
    let root = PathBuf::from(
        ApplicationData::Current()?
            .LocalFolder()?
            .Path()?
            .to_os_string(),
    )
    .join("skathi-notification-routes-v1");
    fs::create_dir_all(&root)?;
    let routes = Routes { root };
    routes.sweep()?;
    let factory: IClassFactory = Factory.into();
    unsafe { CoRegisterClassObject(&clsid, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE) }?;
    Ok((routes, apartment))
}

#[implement(IClassFactory)]
struct Factory;

#[allow(non_snake_case)]
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if out.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            *out = std::ptr::null_mut();
        }
        if outer.as_ref().is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let callback: INotificationActivationCallback = Callback.into();
        unsafe { callback.query(iid, out).ok() }
    }
    fn LockServer(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(INotificationActivationCallback)]
struct Callback;

#[allow(non_snake_case)]
impl INotificationActivationCallback_Impl for Callback_Impl {
    fn Activate(
        &self,
        app: &PCWSTR,
        args: &PCWSTR,
        data: *const NOTIFICATION_USER_INPUT_DATA,
        count: u32,
    ) -> windows::core::Result<()> {
        if app.is_null() || args.is_null() {
            return Ok(());
        }
        let app = unsafe { app.to_string()? };
        if Some(app.as_str()) != sk_permissions::package::app_user_model_id() {
            return Ok(());
        }
        let args = unsafe { args.to_string()? };
        // The toast has at most one input. The COM marshaler owns both strings.
        let mut text = None;
        if count == 1 && !data.is_null() {
            let item = unsafe { &*data };
            if !item.Key.is_null()
                && !item.Value.is_null()
                && unsafe { item.Key.to_string()? } == "reply"
            {
                text = Some(unsafe { item.Value.to_string()? });
            }
        }
        if let Some(routes) = routes() {
            routes.receive(&args, text).map_err(|err| {
                log::warn!("Could not consume notification activation: {err}");
                windows::core::Error::from(windows::Win32::Foundation::E_FAIL)
            })?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    token: String,
    id: String,
    actions: Vec<(String, bool)>,
}

pub(super) struct Routes {
    root: PathBuf,
}

fn valid_key(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

// Keep updates, withdrawal and COM activation ordered across running instances.
// Windows releases the file lock when its handle closes, including on a crash.
impl Routes {
    fn access(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("lock"))?;
        file.lock()?;
        Ok(file)
    }

    fn sweep(&self) -> io::Result<()> {
        let _lock = self.access()?;
        for entry in fs::read_dir(&self.root)?.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| valid_key(name) || name == "pending")
                && entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(7 * 24 * 3600))
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }

    pub(super) fn publish(
        &self,
        n: &Notice,
        send: impl FnOnce(&str) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let _lock = self.access().map_err(io_error)?;
        let token = Uuid::new_v4().simple().to_string();
        let record = Record {
            token: token.clone(),
            id: n.id.clone(),
            actions: n.actions.iter().map(|a| (a.id.clone(), a.reply)).collect(),
        };
        let path = self.root.join(&n.native_id);
        let previous = read_optional(&path).map_err(io_error)?;
        self.write(
            &path,
            &serde_json::to_vec(&record).map_err(|e| Error::operation(e.to_string()))?,
        )
        .map_err(io_error)?;
        if let Err(error) = send(&format!("{}/{token}", n.native_id)) {
            if let Some(bytes) = previous {
                self.write(&path, &bytes).map_err(io_error)?;
            } else {
                fs::remove_file(path).map_err(io_error)?;
            }
            return Err(error);
        }
        Ok(())
    }

    // Atomic replacement keeps a crash from exposing a partial routing record.
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let pending = self.root.join("pending");
        let mut file = File::create(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(pending, path)
    }

    pub(super) fn withdraw(
        &self,
        native_id: &str,
        remove: impl FnOnce() -> Result<(), Error>,
    ) -> Result<(), Error> {
        let _lock = self.access().map_err(io_error)?;
        remove()?;
        match fs::remove_file(self.root.join(native_id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_error(e)),
            _ => Ok(()),
        }
    }

    fn receive(&self, args: &str, text: Option<String>) -> io::Result<()> {
        let mut parts = args.split('/');
        let (Some(key), Some(token), Some(choice), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Ok(());
        };
        if !valid_key(key) || !valid_key(token) {
            return Ok(());
        }
        let _lock = self.access()?;
        let path = self.root.join(key);
        let Some(bytes) = read_optional(&path)? else {
            return Ok(());
        };
        let record: Record = serde_json::from_slice(&bytes)?;
        if token != record.token || key != super::super::native_id(&record.id) {
            return Ok(());
        }
        let action = match choice {
            "open" => None,
            idx => match idx
                .parse::<usize>()
                .ok()
                .and_then(|i| record.actions.get(i))
            {
                Some(action) => Some(action.clone()),
                None => return Ok(()),
            },
        };
        match action {
            Some((action, true)) => {
                let Some(text) = text else {
                    return Ok(());
                };
                fs::remove_file(path)?;
                reply(record.id, action, text);
            }
            other => {
                fs::remove_file(path)?;
                activate(record.id, other.map(|(id, _)| id));
            }
        }
        Ok(())
    }
}

fn io_error(e: io::Error) -> Error {
    Error::operation(e.to_string())
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{CoCreateInstance, CoRevokeClassObject};

    #[test]
    fn registered_factory_produces_the_windows_activation_interface() {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        let _apartment = Apartment;
        let clsid: GUID = Uuid::new_v4().as_u128().into();
        let factory: IClassFactory = Factory.into();
        let cookie = unsafe {
            CoRegisterClassObject(&clsid, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)
        }
        .unwrap();
        let actual: INotificationActivationCallback =
            unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }.unwrap();
        unsafe { actual.Activate(PCWSTR::null(), PCWSTR::null(), &[]) }.unwrap();
        unsafe { CoRevokeClassObject(cookie) }.unwrap();
    }

    #[test]
    fn replacement_rejects_stale_clicks_and_rolls_back_on_submission_failure() {
        let root = std::env::temp_dir().join(format!("sk-toast-routes-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let routes = Routes { root: root.clone() };
        let id = "a".repeat(32);
        let notice = Notice {
            native_id: super::super::super::native_id(&id),
            id,
            title: "Test".into(),
            body: None,
            actions: Vec::new(),
            group: None,
            silent: true,
            image: None,
        };
        let path = root.join(&notice.native_id);
        let mut first = String::new();
        routes
            .publish(&notice, |r| {
                first = r.to_owned();
                Ok(())
            })
            .unwrap();
        let mut second = String::new();
        routes
            .publish(&notice, |r| {
                second = r.to_owned();
                Ok(())
            })
            .unwrap();
        assert_ne!(first, second);
        let stored = fs::read(&path).unwrap();
        routes.receive(&format!("{first}/open"), None).unwrap();
        routes.receive(&format!("{second}/0"), None).unwrap();
        assert!(routes
            .publish(&notice, |_| Err(Error::operation("test refusal")))
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), stored);
        routes.withdraw(&notice.native_id, || Ok(())).unwrap();
        assert!(!path.exists());
        fs::remove_file(root.join("lock")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
