//! Local Windows notifications through the inbox WinRT API.
mod activation;
use super::{activate, reply, Capabilities, Error, Notice, Submission};
pub(crate) use activation::background_launch;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use windows::{
    core::{h, Error as WinError, IInspectable, Interface, HSTRING},
    Data::Xml::Dom::XmlDocument,
    Foundation::{IPropertyValue, TypedEventHandler},
    Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_PATH_NOT_FOUND, E_ACCESSDENIED,
            E_NOINTERFACE, REGDB_E_CLASSNOTREG,
        },
        System::{
            Com::CoTaskMemFree,
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
        },
        UI::Shell::{
            FOLDERID_AppsFolder, GetCurrentProcessExplicitAppUserModelID, IShellItem,
            SHCreateItemInKnownFolder, KF_FLAG_DEFAULT,
        },
    },
    UI::Notifications::{
        NotificationSetting, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
        ToastNotifier,
    },
};

const MAX_ACTIONS: usize = 2;
type Banners = Arc<Mutex<HashMap<String, (Arc<AtomicBool>, ToastNotification)>>>;

fn os_error(err: WinError) -> Error {
    match err.code() {
        E_ACCESSDENIED => Error::denied(err.to_string()),
        REGDB_E_CLASSNOTREG | E_NOINTERFACE => Error::unsupported(err.to_string()),
        _ => Error::operation(err.to_string()),
    }
}

fn explicit_id() -> Option<String> {
    // The shell allocates the string even when the caller only needs to read it.
    unsafe {
        let ptr = GetCurrentProcessExplicitAppUserModelID().ok()?;
        let id = ptr.to_string().ok();
        CoTaskMemFree(Some(ptr.0.cast()));
        id
    }
}

fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.encode_utf16().count() <= 128
        && !id.contains(['\\', '/', '\0'])
        && !id.chars().any(char::is_whitespace)
}

fn posting_id() -> Option<&'static str> {
    static APP_ID: OnceLock<Option<String>> = OnceLock::new();
    APP_ID
        .get_or_init(|| {
            let id = sk_permissions::package::app_user_model_id()
                .map(str::to_owned)
                .or_else(explicit_id)
                .or_else(crate::embed::desktop_id)
                .or_else(|| {
                    sk_desktop::launch::identity().map(|app| {
                        // A development project can inherit a display name containing spaces.
                        if valid_app_id(&app.id) {
                            app.id.clone()
                        } else {
                            format!(
                                "Skathi.{}",
                                &format!("{:x}", Sha256::digest(app.id.as_bytes()))[..32]
                            )
                        }
                    })
                })
                .filter(|id| !id.is_empty());
            if let Some(id) = &id {
                sk_permissions::win::set_aumid(id);
            }
            id
        })
        .as_deref()
}

pub(crate) fn initialize() {
    // The permission watcher subscribes to registry keys once. Select its
    // per-app key before the runtime installs navigator.permissions.
    posting_id();
    activation::initialize();
}

fn prepare_unpackaged_id(id: &str) -> Result<(), Error> {
    if !valid_app_id(id) {
        return Err(Error::unsupported("Invalid Windows application identity"));
    }
    let registered: windows::core::Result<IShellItem> = unsafe {
        SHCreateItemInKnownFolder(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, &HSTRING::from(id))
    };
    if registered.is_ok() {
        return Ok(());
    }
    // Honor an existing machine-wide or per-user registration, including its
    // localized DisplayName and icon. A loose SDK executable has no shortcut.
    let key_path = format!(r"AppUserModelId\{id}");
    let missing = |err: &WinError| {
        [
            ERROR_FILE_NOT_FOUND.to_hresult(),
            ERROR_PATH_NOT_FOUND.to_hresult(),
        ]
        .contains(&err.code())
    };
    match windows_registry::CLASSES_ROOT.open(&key_path) {
        Ok(_) => return Ok(()),
        Err(err) if missing(&err) => {}
        Err(err) => return Err(os_error(err)),
    }
    let key = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\{key_path}"))
        .map_err(os_error)?;
    if let Err(err) = key.get_type("DisplayName") {
        if !missing(&err) {
            return Err(os_error(err));
        }
        let name = sk_desktop::launch::identity()
            .map(|identity| identity.name.as_str())
            .unwrap_or(id);
        key.set_string("DisplayName", name).map_err(os_error)?;
    }
    Ok(())
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

struct Session {
    notifier: ToastNotifier,
    app_id: HSTRING,
    banners: Banners,
    // Drop the WinRT objects before uninitializing their worker thread.
    _apartment: Apartment,
}

pub(super) struct Backend {
    session: Option<Session>,
    images: HashMap<String, PathBuf>,
}

impl Backend {
    pub(super) fn new() -> Self {
        Self {
            session: None,
            images: HashMap::new(),
        }
    }

    fn session(&mut self) -> Result<&Session, Error> {
        if self.session.is_none() {
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(os_error)?;
            let apartment = Apartment;
            let packaged_id = sk_permissions::package::app_user_model_id();
            let app_id = posting_id().ok_or_else(|| {
                Error::unsupported("Windows notifications need an application identity")
            })?;
            if packaged_id.is_none() {
                prepare_unpackaged_id(app_id)?;
            }
            let app_id = HSTRING::from(app_id);
            let notifier = if packaged_id.is_some() {
                ToastNotificationManager::CreateToastNotifier()
            } else {
                ToastNotificationManager::CreateToastNotifierWithId(&app_id)
            }
            .map_err(os_error)?;
            self.session = Some(Session {
                notifier,
                app_id,
                banners: Arc::default(),
                _apartment: apartment,
            });
        }
        Ok(self.session.as_ref().unwrap())
    }

    pub(super) fn capabilities(&mut self) -> Result<Capabilities, Error> {
        self.session()?;
        Ok(Capabilities {
            management: "application",
            activation: if activation::routes().is_some() {
                "relaunch"
            } else {
                "running"
            },
            max_actions: MAX_ACTIONS,
            silent: true,
            grouping: true,
            inline_reply: true,
            image: true,
        })
    }

    pub(super) fn show(&mut self, n: &Notice) -> Result<Submission, Error> {
        let session = self.session()?;
        check_setting(
            session.notifier.Setting(),
            sk_permissions::package::app_user_model_id().is_none(),
            || {
                sk_permissions::win::read_state(sk_permissions::Cap::Notifications)
                    == sk_permissions::State::Granted
            },
        )?;
        let image = n.image.as_ref().map(|image| image.stage()).transpose()?;
        // Each identifier is 32 ASCII hex digits. Two 16-character parts fit the
        // tag/group limits even on older builds without truncating the digest.
        let send = |route: Option<&str>| -> Result<(), Error> {
            let uri = image.as_deref().map(image_uri).transpose()?;
            let document =
                toast_document(n, route, uri.as_ref().map(url::Url::as_str)).map_err(os_error)?;
            let toast = ToastNotification::CreateToastNotification(&document).map_err(os_error)?;
            let (tag, group) = n.native_id.split_at(16);
            toast.SetTag(&tag.into()).map_err(os_error)?;
            toast.SetGroup(&group.into()).map_err(os_error)?;
            if route.is_some() {
                return session.notifier.Show(&toast).map_err(os_error);
            }
            show_banner(session, &toast, n).map_err(os_error)
        };
        let result = if let Some(routes) = activation::routes() {
            routes.publish(n, |route| send(Some(route)))
        } else {
            send(None)
        };
        if let Err(error) = result {
            if let Some(path) = image {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        if let Some(path) = self.images.remove(&n.native_id) {
            let _ = fs::remove_file(path);
        }
        if let Some(path) = image {
            self.images.insert(n.native_id.clone(), path);
        }
        let actions: Vec<_> = n
            .actions
            .iter()
            .take(MAX_ACTIONS)
            .map(|a| a.id.clone())
            .collect();
        Ok(Submission {
            actions,
            ..Default::default()
        })
    }

    pub(super) fn close(&mut self, native_id: &str) -> Result<(), Error> {
        let session = self.session()?;
        let (tag, group) = native_id.split_at(16);
        let remove = || {
            ToastNotificationManager::History()
                .and_then(|history| {
                    history.RemoveGroupedTagWithId(&tag.into(), &group.into(), &session.app_id)
                })
                .map_err(os_error)
        };
        if let Some(routes) = activation::routes() {
            routes.withdraw(native_id, remove)
        } else {
            remove()
        }?;
        session.banners.lock().unwrap().remove(native_id);
        if let Some(path) = self.images.remove(native_id) {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }
}

fn check_setting(
    setting: windows::core::Result<NotificationSetting>,
    unpackaged: bool,
    registered_settings_allow: impl FnOnce() -> bool,
) -> Result<(), Error> {
    match setting {
        Ok(NotificationSetting::Enabled) => {}
        Ok(setting) => {
            return Err(Error::denied(format!(
                "Windows notification setting: {setting:?}"
            )))
        }
        // A newly registered loose app may have no broker settings row yet,
        // even though Show is available. Consult the global, policy and
        // per-app switches before allowing the submission.
        Err(err) if unpackaged && err.code() == ERROR_NOT_FOUND.to_hresult() => {}
        Err(err) => return Err(os_error(err)),
    }
    if registered_settings_allow() {
        Ok(())
    } else {
        Err(Error::denied(
            "Windows has disabled notifications for this application",
        ))
    }
}

fn image_uri(path: &Path) -> Result<url::Url, Error> {
    if sk_permissions::package::app_user_model_id().is_some() {
        let local = windows::Storage::ApplicationData::Current()
            .and_then(|data| data.LocalFolder())
            .and_then(|folder| folder.Path())
            .map_err(os_error)?
            .to_os_string();
        let relative = path
            .strip_prefix(local)
            .map_err(|_| Error::operation("Notification image is outside package LocalState"))?;
        let mut uri = url::Url::parse("ms-appdata:///local").unwrap();
        {
            let mut segments = uri.path_segments_mut().unwrap();
            for name in relative {
                segments.push(&name.to_string_lossy());
            }
        }
        return Ok(uri);
    }
    url::Url::from_file_path(path)
        .map_err(|_| Error::operation("Invalid staged notification image path"))
}

fn show_banner(
    session: &Session,
    toast: &ToastNotification,
    n: &Notice,
) -> windows::core::Result<()> {
    let id = n.id.clone();
    let actions = n.actions.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let finish = {
        let finished = finished.clone();
        let banners = Arc::downgrade(&session.banners);
        let key = n.native_id.clone();
        move || {
            if let Some(banners) = banners.upgrade() {
                let mut banners = banners.lock().unwrap();
                finished.store(true, Ordering::Relaxed);
                if banners
                    .get(&key)
                    .is_some_and(|(instance, _)| Arc::ptr_eq(&finished, instance))
                {
                    banners.remove(&key);
                }
            }
        }
    };
    let dismissed = finish.clone();
    toast.Dismissed(&TypedEventHandler::new(move |_, _| {
        dismissed();
        Ok(())
    }))?;
    let failed = finish.clone();
    toast.Failed(&TypedEventHandler::new(move |_, _| {
        failed();
        Ok(())
    }))?;
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_, args| {
            finish();
            let Some(args) = args.as_ref() else {
                activate(id.clone(), None);
                return Ok(());
            };
            let args = args.cast::<ToastActivatedEventArgs>()?;
            let argument = args.Arguments()?.to_string();
            if argument == "open" || argument.is_empty() {
                activate(id.clone(), None);
            } else if let Some(action) = argument.parse::<usize>().ok().and_then(|i| actions.get(i))
            {
                if action.reply {
                    let text = args
                        .UserInput()?
                        .Lookup(h!("reply"))?
                        .cast::<IPropertyValue>()?
                        .GetString()?;
                    reply(id.clone(), action.id.clone(), text.to_string());
                } else {
                    activate(id.clone(), Some(action.id.clone()));
                }
            }
            Ok(())
        },
    ))?;
    session.notifier.Show(toast)?;
    let mut banners = session.banners.lock().unwrap();
    if finished.load(Ordering::Relaxed) {
        banners.remove(&n.native_id);
    } else {
        banners.insert(n.native_id.clone(), (finished, toast.clone()));
    }
    Ok(())
}

fn toast_document(
    n: &Notice,
    route: Option<&str>,
    image_uri: Option<&str>,
) -> windows::core::Result<XmlDocument> {
    let argument = |choice: &str| -> HSTRING {
        match route {
            Some(base) => format!("{base}/{choice}").into(),
            None => choice.into(),
        }
    };
    let document = XmlDocument::new()?;
    let toast = document.CreateElement(h!("toast"))?;
    toast.SetAttribute(h!("launch"), &argument("open"))?;
    document.AppendChild(&toast)?;
    if let Some(group) = &n.group {
        let header = document.CreateElement(h!("header"))?;
        header.SetAttribute(h!("id"), &group.id.as_str().into())?;
        header.SetAttribute(h!("title"), &group.label.as_str().into())?;
        header.SetAttribute(h!("arguments"), &argument("open"))?;
        toast.AppendChild(&header)?;
    }
    let visual = document.CreateElement(h!("visual"))?;
    toast.AppendChild(&visual)?;
    let binding = document.CreateElement(h!("binding"))?;
    binding.SetAttribute(h!("template"), h!("ToastGeneric"))?;
    visual.AppendChild(&binding)?;
    for text in std::iter::once(n.title.as_str()).chain(n.body.as_deref()) {
        let element = document.CreateElement(h!("text"))?;
        element.SetInnerText(&text.into())?;
        binding.AppendChild(&element)?;
    }
    if !n.actions.is_empty() {
        let actions = document.CreateElement(h!("actions"))?;
        if n.actions.iter().any(|a| a.reply) {
            let input = document.CreateElement(h!("input"))?;
            input.SetAttribute(h!("type"), h!("text"))?;
            input.SetAttribute(h!("id"), h!("reply"))?;
            actions.AppendChild(&input)?;
        }
        for (index, action) in n.actions.iter().take(MAX_ACTIONS).enumerate() {
            let element = document.CreateElement(h!("action"))?;
            element.SetAttribute(h!("content"), &action.label.as_str().into())?;
            element.SetAttribute(h!("arguments"), &argument(&index.to_string()))?;
            if action.reply {
                element.SetAttribute(h!("hint-inputId"), h!("reply"))?;
            }
            actions.AppendChild(&element)?;
        }
        toast.AppendChild(&actions)?;
    }
    if let Some(uri) = image_uri {
        let image = document.CreateElement(h!("image"))?;
        image.SetAttribute(h!("src"), &uri.into())?;
        binding.AppendChild(&image)?;
    }
    if n.silent {
        let audio = document.CreateElement(h!("audio"))?;
        audio.SetAttribute(h!("silent"), h!("true"))?;
        toast.AppendChild(&audio)?;
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_loose_identity_uses_its_permission_until_the_broker_knows_it() {
        let missing = || Err(WinError::from_hresult(ERROR_NOT_FOUND.to_hresult()));
        assert!(check_setting(missing(), true, || true).is_ok());
        assert!(check_setting(missing(), true, || false).is_err());
        assert!(check_setting(missing(), false, || true).is_err());
        assert!(check_setting(Ok(NotificationSetting::Enabled), true, || false).is_err());
        assert!(check_setting(
            Ok(NotificationSetting::DisabledForApplication),
            true,
            || true
        )
        .is_err());
    }

    #[test]
    fn toast_encodes_user_text_and_applies_the_requested_sound() {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        let _apartment = Apartment;
        let mut notice = Notice {
            id: "example".into(),
            native_id: "0123456789abcdef".repeat(2),
            title: "<Example> & friends".into(),
            body: Some("body".into()),
            actions: vec![super::super::Action {
                id: "read".into(),
                label: "Mark \"read\" & close".into(),
                reply: true,
            }],
            silent: true,
            group: Some(super::super::Group {
                id: "topic".into(),
                label: "Updates".into(),
            }),
            image: None,
        };
        let xml =
            toast_document(&notice, Some("native/token"), Some("file:///C:/photo.png")).unwrap();
        for (query, value) in [
            ("//header/@title", "Updates"),
            ("//input/@id", "reply"),
            ("//action/@arguments", "native/token/0"),
            ("//toast/@launch", "native/token/open"),
            ("//image/@src", "file:///C:/photo.png"),
        ] {
            assert_eq!(
                xml.SelectSingleNode(&query.into())
                    .unwrap()
                    .InnerText()
                    .unwrap(),
                value
            );
        }
        assert_eq!(
            xml.SelectSingleNode(h!("//text[1]"))
                .unwrap()
                .InnerText()
                .unwrap()
                .to_string(),
            notice.title
        );
        assert_eq!(
            xml.SelectSingleNode(h!("//action/@content"))
                .unwrap()
                .InnerText()
                .unwrap()
                .to_string(),
            notice.actions[0].label
        );
        assert_eq!(
            xml.SelectSingleNode(h!("//audio/@silent"))
                .unwrap()
                .InnerText()
                .unwrap(),
            "true"
        );
        notice.silent = false;
        assert_eq!(
            toast_document(&notice, None, None)
                .unwrap()
                .GetElementsByTagName(h!("audio"))
                .unwrap()
                .Length()
                .unwrap(),
            0
        );
    }
}
