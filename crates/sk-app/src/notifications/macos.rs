use std::cell::OnceCell;
use std::collections::HashSet;
use std::ptr::NonNull;
use std::sync::{mpsc, Once, OnceLock};
use std::time::{Duration, Instant};

use block2::{DynBlock, RcBlock};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, ClassType};
use objc2_foundation::{
    ns_string, NSArray, NSDictionary, NSError, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSSet, NSString, NSURL,
};
use objc2_user_notifications::*;
use sha2::{Digest, Sha256};
use sk_permissions::mac::{
    fresh_notification_permission, notification_bundle_available, refresh_notification_permission,
};
use sk_permissions::State;

use super::{Capabilities, Error, Notice, Submission};

const SERVICE_TIMEOUT: Duration = Duration::from_secs(10);
static FIRST_ACTION_IN_BACKGROUND: OnceLock<bool> = OnceLock::new();

pub(super) fn background_launch() -> bool {
    FIRST_ACTION_IN_BACKGROUND.get().copied().unwrap_or(false)
}

thread_local! {
    // UNUserNotificationCenter keeps a weak delegate.
    static DELEGATE: OnceCell<Retained<SkathiNotificationDelegate>> = const { OnceCell::new() };
}

pub(super) fn initialize() {
    autoreleasepool(|_| {
        if !notification_bundle_available() {
            return;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        if center.delegate().is_none() {
            DELEGATE.with(|slot| {
                let delegate = slot.get_or_init(|| unsafe {
                    msg_send![SkathiNotificationDelegate::alloc(), init]
                });
                center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
            });
        }
        static OBSERVE: Once = Once::new();
        OBSERVE.call_once(|| {
            refresh_notification_permission();
            let on_focus =
                RcBlock::new(|_: NonNull<NSNotification>| refresh_notification_permission());
            // SAFETY: the default center retains this observer for the process lifetime.
            let _ = unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(ns_string!("NSApplicationDidBecomeActiveNotification")),
                    None,
                    None,
                    &on_focus,
                )
            };
        });
    });
}

fn center() -> Result<Retained<UNUserNotificationCenter>, Error> {
    if !notification_bundle_available() {
        return Err(Error::unsupported(
            "macOS notifications require an application bundle",
        ));
    }
    Ok(UNUserNotificationCenter::currentNotificationCenter())
}

fn handles_activations(center: &UNUserNotificationCenter) -> bool {
    center
        .delegate()
        .is_some_and(|delegate| delegate.isKindOfClass(SkathiNotificationDelegate::class()))
}

pub(super) struct Backend {
    categories_since_sweep: u8,
}

impl Backend {
    pub(super) fn new() -> Self {
        Self {
            categories_since_sweep: 0,
        }
    }

    pub(super) fn capabilities(&mut self) -> Result<Capabilities, Error> {
        autoreleasepool(|_| {
            let Ok(center) = center() else {
                return Ok(Capabilities::default());
            };
            fresh_notification_permission().map_err(Error::operation)?;
            let active = handles_activations(&center);
            Ok(Capabilities {
                management: "application",
                activation: if active { "relaunch" } else { "none" },
                max_actions: if active { 2 } else { 0 },
                inline_reply: active,
                grouping: true,
                image: true,
                silent: true,
            })
        })
    }

    pub(super) fn show(&mut self, notice: &Notice) -> Result<Submission, Error> {
        autoreleasepool(|_| {
            let center = center()?;
            if fresh_notification_permission().map_err(Error::operation)? != State::Granted {
                return Err(Error::denied(
                    "Enable notifications for this app before posting",
                ));
            }
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&notice.title));
            if let Some(body) = &notice.body {
                content.setBody(&NSString::from_str(body));
            }
            if !notice.silent {
                content.setSound(Some(&UNNotificationSound::defaultSound()));
            }
            if let Some(group) = &notice.group {
                content.setThreadIdentifier(&NSString::from_str(&group.id));
            }
            let mut actions = Vec::new();
            if handles_activations(&center) {
                if !notice.actions.is_empty() {
                    let labels = notice
                        .actions
                        .iter()
                        .map(|a| (a.label.clone(), a.reply))
                        .collect();
                    let live = if self.categories_since_sweep == 0 {
                        list_requests(&center, |r| r.content().categoryIdentifier().to_string())
                            .ok()
                    } else {
                        None
                    };
                    self.categories_since_sweep = (self.categories_since_sweep + 1) % 32;
                    let category = register_category(&center, labels, live)?;
                    content.setCategoryIdentifier(&category);
                    actions.extend(notice.actions.iter().map(|a| a.id.clone()));
                }
                let route_actions: Vec<_> =
                    notice.actions.iter().map(|a| (&a.id, a.reply)).collect();
                let route = serde_json::to_string(&(&notice.id, &route_actions))
                    .map_err(|e| Error::operation(e.to_string()))?;
                let value = NSString::from_str(&route);
                let dict = NSDictionary::from_slices(&[ns_string!("skathi.route.v1")], &[&*value]);
                // SAFETY: the untyped userInfo dictionary accepts NSString keys and values.
                unsafe {
                    let dict = Retained::cast_unchecked::<NSDictionary>(dict);
                    content.setUserInfo(&dict);
                }
            }
            let staged = if let Some(image) = &notice.image {
                let path = image.stage()?;
                let attached = NSURL::from_file_path(&path)
                    .ok_or_else(|| Error::operation("Invalid notification attachment file URL"))
                    .and_then(|url| {
                        // SAFETY: a private, fully written PNG file and no untyped attachment options.
                        unsafe {
                            UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(
                                ns_string!("image"),
                                &url,
                                None,
                            )
                        }
                        .map_err(|e| Error::operation(e.localizedDescription().to_string()))
                    });
                match attached {
                    Ok(attachment) => {
                        content.setAttachments(&NSArray::from_retained_slice(&[attachment]))
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(path);
                        return Err(error);
                    }
                }
                Some(path)
            } else {
                None
            };
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&notice.native_id),
                &content,
                None,
            );
            let (send, receive) = mpsc::channel();
            let completion = RcBlock::new(move |error: *mut NSError| {
                // SAFETY: UserNotifications supplies nil or a live error for the callback.
                let result = match unsafe { error.as_ref() } {
                    None => Ok(()),
                    Some(e) => {
                        let msg =
                            format!("{} ({} {})", e.localizedDescription(), e.domain(), e.code());
                        let denied = unsafe { UNErrorDomain } == Some(&*e.domain())
                            && e.code() == UNErrorCode::NotificationsNotAllowed.0;
                        Err(if denied {
                            Error::denied(msg)
                        } else {
                            Error::operation(msg)
                        })
                    }
                };
                let _ = send.send(result);
            });
            center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
            let result = receive
                .recv_timeout(SERVICE_TIMEOUT)
                .map_err(|_| Error::operation("macOS did not acknowledge the notification"))?;
            if let Some(path) = staged {
                let _ = std::fs::remove_file(path);
            }
            result?;
            Ok(Submission {
                actions,
                ..Default::default()
            })
        })
    }

    pub(super) fn close(&mut self, native_id: &str) -> Result<(), Error> {
        autoreleasepool(|_| {
            let center = center()?;
            let ids = NSArray::from_retained_slice(&[NSString::from_str(native_id)]);
            center.removePendingNotificationRequestsWithIdentifiers(&ids);
            center.removeDeliveredNotificationsWithIdentifiers(&ids);
            // The removal methods return before the center processes them.
            let deadline = Instant::now() + SERVICE_TIMEOUT;
            while list_requests(&center, |r| r.identifier().to_string())?.contains(native_id) {
                if Instant::now() >= deadline {
                    return Err(Error::operation(
                        "macOS did not confirm notification removal",
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(())
        })
    }
}

fn register_category(
    center: &UNUserNotificationCenter,
    labels: Vec<(String, bool)>,
    live: Option<HashSet<String>>,
) -> Result<Retained<NSString>, Error> {
    let encoded = serde_json::to_vec(&labels).map_err(|e| Error::operation(e.to_string()))?;
    let key = format!("skathi.buttons.v1.{}", hex::encode(Sha256::digest(encoded)));
    let category_id = NSString::from_str(&key);
    let (send, receive) = mpsc::channel();
    let read = RcBlock::new(move |categories: NonNull<NSSet<UNNotificationCategory>>| {
        let id = NSString::from_str(&key);
        // SAFETY: the OS-retained set is valid until this callback returns.
        let registered = unsafe { categories.as_ref() };
        let mut union = registered.to_vec();
        if let Some(live) = &live {
            union.retain(|category| {
                let existing = category.identifier().to_string();
                existing == key
                    || !existing.starts_with("skathi.buttons.v1.")
                    || live.contains(&existing)
            });
        }
        let mut changed = union.len() != registered.len();
        if !union.iter().any(|category| category.identifier() == id) {
            let actions = labels
                .iter()
                .enumerate()
                .map(|(i, (label, reply))| {
                    let id = NSString::from_str(&format!("skathi.button.{i}"));
                    let label = NSString::from_str(label);
                    if *reply {
                        UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
                            &id, &label, UNNotificationActionOptions::empty(), &label, ns_string!("")
                        ).into_super()
                    } else {
                        UNNotificationAction::actionWithIdentifier_title_options(
                            &id, &label, UNNotificationActionOptions::empty())
                    }
                })
                .collect::<Vec<_>>();
            let category =
                UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                    &id,
                    &NSArray::from_retained_slice(&actions),
                    &NSArray::new(),
                    UNNotificationCategoryOptions::empty(),
                );
            // Registration replaces the entire application-wide set, including the embedder's categories.
            union.push(category);
            changed = true;
        }
        if changed {
            UNUserNotificationCenter::currentNotificationCenter()
                .setNotificationCategories(&NSSet::from_retained_slice(&union));
        }
        let _ = send.send(());
    });
    center.getNotificationCategoriesWithCompletionHandler(&read);
    receive
        .recv_timeout(SERVICE_TIMEOUT)
        .map_err(|_| Error::operation("macOS did not return notification categories"))?;
    Ok(category_id)
}

fn list_requests(
    center: &UNUserNotificationCenter,
    select: fn(&UNNotificationRequest) -> String,
) -> Result<HashSet<String>, Error> {
    let (send, receive) = mpsc::channel();
    let pending = RcBlock::new(move |requests: NonNull<NSArray<UNNotificationRequest>>| {
        // SAFETY: the array is owned by the center for the duration of the block.
        let ids: HashSet<_> = unsafe { requests.as_ref() }
            .iter()
            .map(|r| select(&r))
            .collect();
        let _ = send.send(ids);
    });
    center.getPendingNotificationRequestsWithCompletionHandler(&pending);
    let mut live = receive
        .recv_timeout(SERVICE_TIMEOUT)
        .map_err(|_| Error::operation("macOS did not return pending notifications"))?;
    let (send, receive) = mpsc::channel();
    let delivered = RcBlock::new(move |cards: NonNull<NSArray<UNNotification>>| {
        // SAFETY: the array is owned by the center for the duration of the block.
        let ids: Vec<_> = unsafe { cards.as_ref() }
            .iter()
            .map(|c| select(&c.request()))
            .collect();
        let _ = send.send(ids);
    });
    center.getDeliveredNotificationsWithCompletionHandler(&delivered);
    live.extend(
        receive
            .recv_timeout(SERVICE_TIMEOUT)
            .map_err(|_| Error::operation("macOS did not return delivered notifications"))?,
    );
    Ok(live)
}

fn receive_activation(response: &UNNotificationResponse) -> Option<()> {
    let request = response.notification().request();
    let info = request.content().userInfo();
    let route = info
        .objectForKey(ns_string!("skathi.route.v1"))?
        .downcast::<NSString>()
        .ok()?;
    let (id, actions): (String, Vec<(String, bool)>) =
        serde_json::from_str(&route.to_string()).ok()?;
    if request.identifier().to_string() != super::native_id(&id) {
        return None;
    }
    let action = response.actionIdentifier();
    if &*action == unsafe { UNNotificationDefaultActionIdentifier } {
        let _ = FIRST_ACTION_IN_BACKGROUND.set(false);
        super::activate(id, None);
    } else {
        let n: usize = action
            .to_string()
            .strip_prefix("skathi.button.")?
            .parse()
            .ok()?;
        let (action_id, expects_text) = actions.get(n)?;
        if *expects_text {
            let text = response
                .downcast_ref::<UNTextInputNotificationResponse>()?
                .userText()
                .to_string();
            let _ = FIRST_ACTION_IN_BACKGROUND.set(true);
            super::reply(id, action_id.clone(), text);
        } else {
            let _ = FIRST_ACTION_IN_BACKGROUND.set(true);
            super::activate(id, Some(action_id.clone()));
        }
    }
    Some(())
}

define_class!(
    // SAFETY: NSObject has no additional subclassing requirements.
    #[unsafe(super(NSObject))]
    struct SkathiNotificationDelegate;

    unsafe impl NSObjectProtocol for SkathiNotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for SkathiNotificationDelegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn received(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            receive_activation(response);
            completion.call(());
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn foreground(
            &self,
            _center: &UNUserNotificationCenter,
            card: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            if card
                .request()
                .content()
                .userInfo()
                .objectForKey(ns_string!("skathi.route.v1"))
                .is_none()
            {
                completion.call((UNNotificationPresentationOptions::empty(),));
                return;
            }
            let mut options =
                UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List;
            if card.request().content().sound().is_some() {
                options |= UNNotificationPresentationOptions::Sound;
            }
            completion.call((options,));
        }
    }
);
