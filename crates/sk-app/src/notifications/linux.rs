use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::future::Future;
use std::io::{self, Cursor, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{
    executor::block_on,
    future::{select, AbortHandle, Abortable, Either},
    StreamExt,
};
use zbus::{
    message::Type,
    zvariant::{OwnedValue, Value},
    Connection, MatchRule, Message, MessageStream,
};

use super::{activate, open_with_token, Capabilities, Error, Notice, Submission};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const NOTIFICATION: &str = "org.freedesktop.portal.Notification";
const DAEMON: &str = "org.freedesktop.Notifications";
const DAEMON_PATH: &str = "/org/freedesktop/Notifications";
const SERVICE_TIMEOUT: Duration = Duration::from_secs(10);
type Dict<'a> = HashMap<&'a str, Value<'a>>;

async fn with_timeout<T>(future: impl Future<Output = zbus::Result<T>>) -> zbus::Result<T> {
    match select(
        std::pin::pin!(future),
        async_io::Timer::after(SERVICE_TIMEOUT),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "The notification service did not answer over D-Bus",
        )
        .into()),
    }
}

struct DirectCard {
    logical_id: String,
    native_id: String,
    buttons: HashMap<String, String>,
    token: Option<String>,
    has_default_action: bool,
}

#[derive(Default)]
struct DirectCards {
    by_number: HashMap<u32, DirectCard>,
    by_name: HashMap<String, u32>,
    // A daemon can emit ActionInvoked or NotificationClosed before the Notify reply.
    in_flight: bool,
    early: Vec<Message>,
}

impl DirectCards {
    fn forget(&mut self, number: u32) {
        if let Some(card) = self.by_number.remove(&number) {
            self.by_name.remove(&card.native_id);
        }
    }

    fn signal(&mut self, msg: Message) {
        if self.in_flight {
            self.early.push(msg);
            return;
        }
        match msg.header().member().map(|m| m.as_str()) {
            Some("ActivationToken") => {
                if let Ok((number, token)) = msg.body().deserialize::<(u32, String)>() {
                    if let Some(card) = self.by_number.get_mut(&number) {
                        card.token = Some(token);
                    }
                }
            }
            Some("NotificationClosed") => {
                if let Ok((number, _reason)) = msg.body().deserialize::<(u32, u32)>() {
                    self.forget(number);
                }
            }
            Some("ActionInvoked") => {
                let Ok((number, key)) = msg.body().deserialize::<(u32, String)>() else {
                    return;
                };
                let Some(card) = self.by_number.get_mut(&number) else {
                    return;
                };
                let token = card.token.take();
                if key == "default" && card.has_default_action {
                    open_with_token(card.logical_id.clone(), token);
                } else if let Some(action) = card.buttons.get(&key) {
                    activate(card.logical_id.clone(), Some(action.clone()));
                }
            }
            _ => {}
        }
    }

    fn finish(&mut self) {
        self.in_flight = false;
        for signal in std::mem::take(&mut self.early) {
            self.signal(signal);
        }
    }
}

#[derive(Clone)]
enum Service {
    Portal { version: u32, inline_reply: bool },
    Direct { features: Vec<String> },
}

impl Service {
    fn portal(&self) -> bool {
        matches!(self, Self::Portal { .. })
    }
    fn supports(&self, flag: &str) -> bool {
        matches!(self, Self::Direct { features } if features.iter().any(|s| s == flag))
    }
    fn silent(&self) -> bool {
        matches!(self, Self::Portal { version: 2.., .. }) || self.supports("sound")
    }
    fn inline_reply(&self) -> bool {
        matches!(
            self,
            Self::Portal {
                inline_reply: true,
                ..
            }
        )
    }
}

struct Session {
    owner: String,
    service: Service,
    stable_identity: bool,
    cards: Arc<Mutex<DirectCards>>,
    signals: AbortHandle,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.signals.abort();
    }
}

pub(super) struct Backend {
    bus: Option<Connection>,
    session: Option<Session>,
    retired_direct: Option<Session>,
    revision: u64,
    // A portal card can outlive the service connection, even after a lost acknowledgement.
    portal_ids: HashSet<String>,
}

impl Backend {
    pub(super) fn new() -> Self {
        Self {
            bus: None,
            session: None,
            retired_direct: None,
            revision: 0,
            portal_ids: HashSet::new(),
        }
    }

    fn refresh(&mut self) -> Result<bool, Error> {
        if !super::on_wayland() {
            return Ok(false);
        }
        if self.bus.is_none() {
            match block_on(with_timeout(Connection::session())) {
                Ok(bus) => self.bus = Some(bus),
                Err(error) if matches!(&error, zbus::Error::InputOutput(io) if io.kind() == io::ErrorKind::TimedOut) =>
                {
                    return Err(bus_error(error));
                }
                Err(_) => return Ok(false),
            }
        }
        let bus = self.bus.as_ref().unwrap();
        let registered_owner = if ashpd::is_sandboxed() {
            None
        } else {
            block_on(with_timeout(register_host(bus, self.session.as_ref()))).map_err(bus_error)?
        };
        let found = match block_on(with_timeout(discover(bus))) {
            Ok(found) => found,
            Err(e) => {
                if matches!(&e, zbus::Error::InputOutput(io) if io.kind() != io::ErrorKind::TimedOut)
                    || matches!(&e, zbus::Error::Connection(..))
                {
                    self.session = None;
                    self.retired_direct = None;
                    self.bus = None;
                }
                return Err(bus_error(e));
            }
        };
        let Some((owner, service)) = found else {
            self.session = None;
            return Ok(false);
        };
        if let Some(s) = &mut self.session {
            if s.owner == owner && s.service.portal() == service.portal() {
                s.service = service;
                return Ok(true);
            }
        }
        let portal = service.portal();
        if portal {
            // The portal may appear while a fallback card is live. Keep its original
            // removal handle and signal subscription, but send all new posts through the portal.
            if let Some(old) = self
                .session
                .take()
                .filter(|s| !s.service.portal() && !s.cards.lock().unwrap().by_name.is_empty())
            {
                self.retired_direct = Some(old);
            }
        } else if let Some(mut old) = self.retired_direct.take().filter(|s| s.owner == owner) {
            old.service = service;
            self.session = Some(old);
            return Ok(true);
        }
        let cards = Arc::new(Mutex::new(DirectCards::default()));
        let signals = block_on(with_timeout(listen(bus, &owner, portal, cards.clone())))
            .map_err(bus_error)?;
        let stable_identity =
            portal && (ashpd::is_sandboxed() || registered_owner.as_ref() == Some(&owner));
        self.session = Some(Session {
            owner,
            service,
            cards,
            signals,
            stable_identity,
        });
        Ok(true)
    }

    pub(super) fn capabilities(&mut self) -> Result<Capabilities, Error> {
        if !self.refresh()? {
            return Ok(Capabilities::default());
        }
        let s = self.session.as_ref().unwrap();
        let actions = s.service.portal() || s.service.supports("actions");
        Ok(Capabilities {
            management: if s.stable_identity {
                "application"
            } else {
                "session"
            },
            activation: if actions { "running" } else { "none" },
            max_actions: if actions { 2 } else { 0 },
            silent: s.service.silent(),
            inline_reply: s.service.inline_reply(),
            image: s.service.portal(),
            ..Default::default()
        })
    }

    fn check_direct_route(&self, native_id: &str) -> Result<(), Error> {
        if self.portal_ids.contains(native_id) {
            return Err(Error::unsupported(
                "Reconnect to the notification portal to manage this notification.",
            ));
        }
        Ok(())
    }

    pub(super) fn show(&mut self, n: &Notice) -> Result<Submission, Error> {
        if !self.refresh()? {
            return Err(Error::unsupported(
                "No notification service is available in this Wayland session.",
            ));
        }
        let bus = self.bus.as_ref().unwrap();
        let s = self.session.as_ref().unwrap();
        if n.silent && !s.service.silent() {
            return Err(Error::unsupported(
                "This notification service does not support silent notifications.",
            ));
        }
        if s.service.portal() {
            let (mut fields, actions) = portal_fields(n, &s.service);
            if let Some(image) = &n.image {
                fields.insert("icon", portal_icon(&image.png, &s.service)?);
            }
            let result = block_on(with_timeout(bus.call_method(
                Some(s.owner.as_str()),
                PORTAL_PATH,
                Some(NOTIFICATION),
                "AddNotification",
                &(&n.native_id, fields),
            )));
            remember_portal_result(&mut self.portal_ids, &n.native_id, &result);
            result.map_err(bus_error)?;
            if let Some(old) = &self.retired_direct {
                close_direct(bus, old, &n.native_id)?;
            }
            return Ok(Submission {
                actions,
                group_omitted: n.group.is_some(),
                ..Default::default()
            });
        }
        self.check_direct_route(&n.native_id)?;
        self.revision += 1;
        let actions_allowed = s.service.supports("actions");
        let mut wire = Vec::new();
        let mut buttons = HashMap::new();
        if actions_allowed {
            wire.extend(["default".to_owned(), String::new()]);
            for (i, action) in n.actions.iter().filter(|a| !a.reply).enumerate() {
                let key = format!("sk-{}-{i}", self.revision);
                wire.extend([key.clone(), action.label.clone()]);
                buttons.insert(key, action.id.clone());
            }
        }
        let mut hints = Dict::new();
        let identity = desktop_id();
        if let Some(id) = &identity {
            hints.insert("desktop-entry", Value::from(id.as_str()));
        }
        if n.silent {
            hints.insert("suppress-sound", Value::from(true));
        }
        let body_omitted = n.body.is_some() && !s.service.supports("body");
        let body = if body_omitted {
            ""
        } else {
            n.body.as_deref().unwrap_or_default()
        };
        let body = if s.service.supports("body-markup") {
            escape_markup(body)
        } else {
            body.to_owned()
        };
        let name = sk_desktop::launch::identity()
            .map(|v| v.name.as_str())
            .unwrap_or_default();
        let replacing = {
            let mut cards = s.cards.lock().unwrap();
            cards.in_flight = true;
            cards.by_name.get(&n.native_id).copied().unwrap_or(0)
        };
        let result = block_on(with_timeout(bus.call_method(
            Some(s.owner.as_str()),
            DAEMON_PATH,
            Some(DAEMON),
            "Notify",
            &(name, replacing, "", &n.title, body, wire, hints, -1_i32),
        )))
        .and_then(|msg| msg.body().deserialize::<u32>());
        let mut cards = s.cards.lock().unwrap();
        if let Ok(number) = result {
            cards.forget(replacing);
            cards.forget(number);
            cards.by_number.insert(
                number,
                DirectCard {
                    logical_id: n.id.clone(),
                    native_id: n.native_id.clone(),
                    buttons,
                    token: None,
                    has_default_action: actions_allowed,
                },
            );
            cards.by_name.insert(n.native_id.clone(), number);
        }
        cards.finish();
        result.map_err(bus_error)?;
        Ok(Submission {
            actions: n
                .actions
                .iter()
                .filter(|a| actions_allowed && !a.reply)
                .map(|a| a.id.clone())
                .collect(),
            body_omitted,
            group_omitted: n.group.is_some(),
            image_omitted: n.image.is_some(),
        })
    }

    pub(super) fn close(&mut self, native_id: &str) -> Result<(), Error> {
        if !self.refresh()? {
            let mut closed_direct = false;
            if let (Some(bus), Some(old)) = (&self.bus, &self.retired_direct) {
                closed_direct = close_direct(bus, old, native_id)?;
            }
            self.check_direct_route(native_id)?;
            if closed_direct {
                return Ok(());
            }
            return Err(Error::unsupported(
                "No notification service is available in this Wayland session.",
            ));
        }
        let bus = self.bus.as_ref().unwrap();
        let s = self.session.as_ref().unwrap();
        if s.service.portal() {
            let portal_result = block_on(with_timeout(bus.call_method(
                Some(s.owner.as_str()),
                PORTAL_PATH,
                Some(NOTIFICATION),
                "RemoveNotification",
                &(native_id,),
            )));
            if portal_result.is_ok() {
                self.portal_ids.remove(native_id);
            }
            if let Some(old) = &self.retired_direct {
                close_direct(bus, old, native_id)?;
            }
            portal_result.map_err(bus_error)?;
        } else {
            close_direct(bus, s, native_id)?;
            self.check_direct_route(native_id)?;
        }
        Ok(())
    }
}

fn remember_portal_result(
    portal_ids: &mut HashSet<String>,
    native_id: &str,
    result: &zbus::Result<Message>,
) {
    if !result.as_ref().is_err_and(confirmed_rejection) {
        portal_ids.insert(native_id.to_owned());
    }
}

fn close_direct(bus: &Connection, s: &Session, native_id: &str) -> Result<bool, Error> {
    let number = s.cards.lock().unwrap().by_name.get(native_id).copied();
    if let Some(number) = number {
        let result = block_on(with_timeout(bus.call_method(
            Some(s.owner.as_str()),
            DAEMON_PATH,
            Some(DAEMON),
            "CloseNotification",
            &(number,),
        )));
        let mut cards = s.cards.lock().unwrap();
        if cards.by_number.contains_key(&number) {
            // The FDO protocol uses an empty error reply for an already expired ID.
            if let Err(e) = result {
                let absent = matches!(&e, zbus::Error::MethodError(name, None, msg)
                            if msg.header().sender().is_some_and(|sender| sender.as_str() == s.owner)
                              && msg.body().is_empty() && matches!(name.as_str(),
                                  "org.freedesktop.DBus.Error.Failed" | "org.freedesktop.DBus.Error.InvalidArgs"));
                let daemon_gone = matches!(&e, zbus::Error::MethodError(name, _, _)
                            if matches!(name.as_str(), "org.freedesktop.DBus.Error.NameHasNoOwner" | "org.freedesktop.DBus.Error.ServiceUnknown"));
                if !absent && !daemon_gone {
                    return Err(bus_error(e));
                }
            }
            cards.forget(number);
        }
    }
    Ok(number.is_some())
}

fn desktop_id() -> Option<String> {
    crate::embed::desktop_id().or_else(|| sk_desktop::launch::identity().map(|v| v.id.clone()))
}

async fn register_host(
    bus: &Connection,
    previous: Option<&Session>,
) -> zbus::Result<Option<String>> {
    let Some(id) = desktop_id() else {
        return Ok(None);
    };
    if let Some(s) = previous.filter(|s| s.service.portal()) {
        let owner = bus
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(PORTAL,),
            )
            .await
            .and_then(|m| m.body().deserialize::<String>());
        if owner.as_ref().ok() == Some(&s.owner) {
            return Ok(s.stable_identity.then(|| s.owner.clone()));
        }
    }
    // The host registry must run on this connection before any portal method.
    let reply = bus
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some("org.freedesktop.host.portal.Registry"),
            "Register",
            &(id, Dict::new()),
        )
        .await;
    Ok(reply
        .ok()
        .and_then(|m| m.header().sender().map(ToString::to_string)))
}

async fn discover(bus: &Connection) -> zbus::Result<Option<(String, Service)>> {
    let probe = bus
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(NOTIFICATION, "version"),
        )
        .await;
    match probe {
        Ok(msg) => {
            let version = u32::try_from(msg.body().deserialize::<OwnedValue>()?)?;
            let owner = msg
                .header()
                .sender()
                .ok_or(zbus::Error::MissingField)?
                .to_string();
            let inline_reply = version >= 2 && portal_supports_reply(bus, &owner).await?;
            Ok(Some((
                owner,
                Service::Portal {
                    version,
                    inline_reply,
                },
            )))
        }
        Err(e) if missing(&e) => {
            if ashpd::is_sandboxed()
                || std::env::var_os("SNAP").is_some()
                || std::env::var_os("FLATPAK_ID").is_some()
            {
                return Ok(None);
            }
            discover_direct(bus).await
        }
        Err(e) => Err(e),
    }
}

fn confirmed_rejection(error: &zbus::Error) -> bool {
    matches!(error, zbus::Error::MethodError(name, _, _) if matches!(name.as_str(),
        "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.portal.Error.NotAllowed" |
        "org.freedesktop.DBus.Error.InvalidArgs" | "org.freedesktop.portal.Error.InvalidArgument" |
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner"))
}

async fn discover_direct(bus: &Connection) -> zbus::Result<Option<(String, Service)>> {
    let msg = match bus
        .call_method(
            Some(DAEMON),
            DAEMON_PATH,
            Some(DAEMON),
            "GetCapabilities",
            &(),
        )
        .await
    {
        Ok(msg) => msg,
        Err(e) if missing(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let features = msg.body().deserialize()?;
    let owner = msg
        .header()
        .sender()
        .ok_or(zbus::Error::MissingField)?
        .to_string();
    Ok(Some((owner, Service::Direct { features })))
}

fn missing(error: &zbus::Error) -> bool {
    matches!(error, zbus::Error::MethodError(name, Some(reason), _) if name.as_str() == "org.freedesktop.DBus.Error.InvalidArgs" && reason.contains("No such interface") && reason.contains(NOTIFICATION))
        || matches!(error, zbus::Error::MethodError(name, _, _) if matches!(name.as_str(),
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" |
        "org.freedesktop.DBus.Error.UnknownInterface" | "org.freedesktop.DBus.Error.UnknownObject" |
        "org.freedesktop.DBus.Error.UnknownProperty" | "org.freedesktop.DBus.Error.UnknownMethod"))
}

fn bus_error(error: zbus::Error) -> Error {
    if matches!(&error, zbus::Error::MethodError(name, _, _) if matches!(name.as_str(),
        "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.portal.Error.NotAllowed"))
    {
        Error::denied(error.to_string())
    } else {
        Error::operation(error.to_string())
    }
}

fn escape_markup(body: &str) -> String {
    body.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn portal_icon(png: &[u8], service: &Service) -> Result<Value<'static>, Error> {
    use image::{imageops, DynamicImage, ImageFormat, RgbaImage};

    // Some notification portals still enforce a 128px square. Keep the image's aspect ratio.
    let decoded = image::load_from_memory_with_format(png, ImageFormat::Png)
        .map_err(|e| Error::operation(e.to_string()))?;
    let edge = decoded.width().max(decoded.height()).min(128);
    let thumbnail = decoded.thumbnail(edge, edge).to_rgba8();
    let mut square = RgbaImage::new(edge, edge);
    imageops::overlay(
        &mut square,
        &thumbnail,
        ((edge - thumbnail.width()) / 2).into(),
        ((edge - thumbnail.height()) / 2).into(),
    );
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(square)
        .write_to(&mut encoded, ImageFormat::Png)
        .map_err(|e| Error::operation(e.to_string()))?;
    if matches!(service, Service::Portal { version: 2.., .. }) {
        let fd = sealed_icon(encoded.get_ref()).map_err(|e| Error::operation(e.to_string()))?;
        let fd = zbus::zvariant::Fd::from(fd);
        Ok(Value::from(("file-descriptor", Value::from(fd))))
    } else {
        Ok(Value::from(("bytes", Value::from(encoded.into_inner()))))
    }
}

fn sealed_icon(png: &[u8]) -> std::io::Result<OwnedFd> {
    // SAFETY: the name is a valid C string. A successful call transfers one new fd.
    let fd = unsafe {
        libc::memfd_create(
            c"skathi-notification".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(png)?;
    file.rewind()?;
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    // SAFETY: file owns the fd, F_ADD_SEALS accepts an integer bitmask.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file.into())
}

async fn portal_supports_reply(bus: &Connection, owner: &str) -> zbus::Result<bool> {
    let msg = match bus
        .call_method(
            Some(owner),
            PORTAL_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(NOTIFICATION, "SupportedOptions"),
        )
        .await
    {
        Ok(msg) => msg,
        Err(e) if missing(&e) => return Ok(false),
        Err(e) => return Err(e),
    };
    let mut options =
        HashMap::<String, OwnedValue>::try_from(msg.body().deserialize::<OwnedValue>()?)?;
    let mut has = |key: &str, value: &str| {
        options
            .remove(key)
            .and_then(|v| Vec::<String>::try_from(v).ok())
            .is_some_and(|values| values.iter().any(|v| v == value))
    };
    Ok(has("category", "im.received") && has("button-purpose", "im.reply-with-text"))
}

fn portal_fields<'a>(n: &'a Notice, service: &Service) -> (Dict<'a>, Vec<String>) {
    let mut fields = Dict::from([
        ("title", Value::from(n.title.as_str())),
        ("default-action", Value::from("sk-open")),
        ("default-action-target", Value::from(n.id.as_str())),
    ]);
    if let Some(body) = &n.body {
        fields.insert("body", Value::from(body.as_str()));
    }
    if service.silent() {
        fields.insert(
            "sound",
            Value::from(if n.silent { "silent" } else { "default" }),
        );
    }
    let mut actions = Vec::new();
    let mut buttons = Vec::new();
    for (index, a) in n
        .actions
        .iter()
        .filter(|a| !a.reply || service.inline_reply())
        .enumerate()
    {
        // Some portal backends also use the action name as the native button key.
        let name = if a.reply {
            "sk-reply"
        } else if index == 0 {
            "sk-button"
        } else {
            "sk-button-2"
        };
        let mut button = Dict::from([
            ("action", Value::from(name)),
            ("label", Value::from(a.label.as_str())),
            ("target", Value::from((n.id.as_str(), a.id.as_str()))),
        ]);
        if a.reply {
            fields.insert("category", Value::from("im.received"));
            button.insert("purpose", Value::from("im.reply-with-text"));
        }
        buttons.push(button);
        actions.push(a.id.clone());
    }
    if !buttons.is_empty() {
        fields.insert("buttons", Value::from(buttons));
    }
    (fields, actions)
}

async fn listen(
    bus: &Connection,
    owner: &str,
    portal: bool,
    cards: Arc<Mutex<DirectCards>>,
) -> zbus::Result<AbortHandle> {
    let (path, interface) = if portal {
        (PORTAL_PATH, NOTIFICATION)
    } else {
        (DAEMON_PATH, DAEMON)
    };
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(owner)?
        .path(path)?
        .interface(interface)?
        .build();
    let mut stream = MessageStream::for_match_rule(rule, bus, None).await?;
    let (abort, registration) = AbortHandle::new_pair();
    let task = async move {
        while let Some(Ok(msg)) = stream.next().await {
            if portal {
                portal_signal(&msg);
            } else {
                cards.lock().unwrap().signal(msg);
            }
        }
    };
    bus.executor()
        .spawn(Abortable::new(task, registration), "notification-signals")
        .detach();
    Ok(abort)
}

fn portal_signal(msg: &Message) {
    if msg
        .header()
        .member()
        .is_none_or(|m| m.as_str() != "ActionInvoked")
    {
        return;
    }
    let Ok((native_id, name, parameters)) = msg
        .body()
        .deserialize::<(String, String, Vec<OwnedValue>)>()
    else {
        return;
    };
    let mut parameters = parameters.into_iter();
    let Some(target) = parameters.next() else {
        return;
    };
    match name.as_str() {
        "sk-open" => {
            if let Ok(id) = String::try_from(target) {
                if super::native_id(&id) != native_id {
                    return;
                }
                let token = parameters
                    .next()
                    .and_then(|p| HashMap::<String, OwnedValue>::try_from(p).ok())
                    .and_then(|mut p| p.remove("activation-token"))
                    .and_then(|v| String::try_from(v).ok());
                open_with_token(id, token);
            }
        }
        "sk-button" | "sk-button-2" => {
            if let Ok((id, action)) = <(String, String)>::try_from(target) {
                if super::native_id(&id) != native_id {
                    return;
                }
                activate(id, Some(action));
            }
        }
        "sk-reply" => {
            if let (Ok((id, action)), Some(text)) = (
                <(String, String)>::try_from(target),
                parameters.nth(1).and_then(|v| String::try_from(v).ok()),
            ) {
                if super::native_id(&id) != native_id {
                    return;
                }
                super::reply(id, action, text);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_submission_that_may_have_succeeded_blocks_only_its_own_direct_route() {
        let mut backend = Backend::new();
        let timeout = Err(io::Error::from(io::ErrorKind::TimedOut).into());
        remember_portal_result(&mut backend.portal_ids, "uncertain", &timeout);

        let call = Message::method_call(PORTAL_PATH, "AddNotification")
            .unwrap()
            .build(&())
            .unwrap();
        let reply = Message::error(&call.header(), "org.freedesktop.DBus.Error.AccessDenied")
            .unwrap()
            .build(&("Not allowed",))
            .unwrap();
        remember_portal_result(&mut backend.portal_ids, "denied", &Err(reply.into()));

        assert!(backend.check_direct_route("uncertain").is_err());
        assert!(backend.check_direct_route("denied").is_ok());
        assert!(backend.check_direct_route("fresh").is_ok());
    }

    #[test]
    fn a_close_signal_received_before_the_notify_reply_retires_both_keys() {
        let mut cards = DirectCards {
            in_flight: true,
            ..Default::default()
        };
        let closed = Message::signal(DAEMON_PATH, DAEMON, "NotificationClosed")
            .unwrap()
            .build(&(42_u32, 2_u32))
            .unwrap();
        cards.signal(closed);
        cards.by_number.insert(
            42,
            DirectCard {
                logical_id: "chat".into(),
                native_id: "abc".into(),
                buttons: HashMap::new(),
                token: None,
                has_default_action: false,
            },
        );
        cards.by_name.insert("abc".into(), 42);
        cards.finish();
        assert!(cards.by_number.is_empty());
        assert!(cards.by_name.is_empty());
    }

    #[test]
    fn portal_serializes_icons_with_the_expected_variant_and_size() {
        let mut png = Cursor::new(Vec::new());
        image::RgbaImage::new(240, 35)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        for version in [1, 2] {
            let icon = portal_icon(
                png.get_ref(),
                &Service::Portal {
                    version,
                    inline_reply: false,
                },
            )
            .unwrap();
            let Value::Structure(structure) = icon else {
                panic!("expected (sv)")
            };
            let [Value::Str(kind), Value::Value(data)] = structure.fields() else {
                panic!("expected (sv)")
            };
            let decoded = match (kind.as_str(), data.as_ref()) {
                ("bytes", Value::Array(bytes)) if version == 1 => {
                    let bytes = Vec::<u8>::try_from(bytes.try_clone().unwrap()).unwrap();
                    image::load_from_memory(&bytes).unwrap()
                }
                ("file-descriptor", Value::Fd(fd)) if version == 2 => {
                    use std::os::fd::AsFd;
                    let file = File::from(fd.as_fd().try_clone_to_owned().unwrap());
                    // SAFETY: F_GET_SEALS takes a valid fd and no third argument.
                    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
                    assert_ne!(seals & libc::F_SEAL_WRITE, 0);
                    image::ImageReader::with_format(
                        std::io::BufReader::new(file),
                        image::ImageFormat::Png,
                    )
                    .decode()
                    .unwrap()
                }
                other => panic!("unexpected portal icon: {other:?}"),
            };
            assert_eq!((decoded.width(), decoded.height()), (128, 128));
        }
    }
}
