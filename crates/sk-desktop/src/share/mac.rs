//! An `NSSharingServicePicker` shown over the owner window. The share settles when the chosen service
//! reports back; dismissing the picker is a cancel.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{NSSharingService, NSSharingServiceDelegate, NSSharingServicePicker, NSSharingServicePickerDelegate};
use objc2_foundation::{
    NSArray, NSError, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSUserCancelledError, NSURL,
};

use super::{write, Request};
use crate::Fail;

struct Ivars {
    token: u32,
    title: String,
    done: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Delegate does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SkathiShareDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSSharingServicePickerDelegate for Delegate {
        #[unsafe(method_id(sharingServicePicker:delegateForSharingService:))]
        fn delegate_for(
            &self,
            _picker: &NSSharingServicePicker,
            _service: &NSSharingService,
        ) -> Option<Retained<ProtocolObject<dyn NSSharingServiceDelegate>>> {
            Some(ProtocolObject::from_retained(self.retain()))
        }

        #[unsafe(method(sharingServicePicker:didChooseSharingService:))]
        fn did_choose(&self, _picker: &NSSharingServicePicker, service: Option<&NSSharingService>) {
            match service {
                Some(s) => s.setSubject(Some(&NSString::from_str(&self.ivars().title))),
                None => self.finish(Err(Fail::new("AbortError", "Share canceled"))),
            }
        }
    }

    unsafe impl NSSharingServiceDelegate for Delegate {
        #[unsafe(method(sharingService:didShareItems:))]
        fn did_share(&self, _service: &NSSharingService, _items: &NSArray) {
            self.finish(Ok("{}".into()));
        }

        #[unsafe(method(sharingService:didFailToShareItems:error:))]
        fn did_fail(&self, _service: &NSSharingService, _items: &NSArray, error: &NSError) {
            let f = if error.code() == NSUserCancelledError {
                Fail::new("AbortError", "Share canceled")
            } else {
                Fail::new("DataError", format!("The share target could not share the data ({})", error.localizedDescription()))
            };
            self.finish(Err(f));
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, token: u32, title: String) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { token, title, done: Cell::new(false) });
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, result: Result<String, Fail>) {
        if !self.ivars().done.replace(true) {
            crate::settle(self.ivars().token, result);
        }
    }
}

thread_local! {
    /// The picker on screen and its delegate, which AppKit only holds weakly.
    static LIVE: RefCell<Option<(Retained<NSSharingServicePicker>, Retained<Delegate>)>> = const { RefCell::new(None) };
}

fn items(req: &Request, files: &[std::path::PathBuf]) -> Retained<NSArray> {
    let mut out: Vec<Retained<AnyObject>> = Vec::new();
    if let Some(t) = &req.text {
        out.push(NSString::from_str(t).into_super().into_super());
    }
    if let Some(u) = req.url.as_deref().and_then(|u| NSURL::URLWithString(&NSString::from_str(u))) {
        out.push(u.into_super().into_super());
    }
    out.extend(files.iter().filter_map(NSURL::from_file_path).map(|u| u.into_super().into_super()));
    NSArray::from_retained_slice(&out)
}

/// The token settled when the chosen service finishes or the picker is dismissed.
pub fn start(owner: isize, req: Request) -> Result<u32, Fail> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(Fail::new("NotAllowedError", "The share sheet can only be shown from the main thread."));
    };
    let view = crate::dialog::window_of(mtm, owner).filter(|w| w.isVisible()).and_then(|w| w.contentView());
    let Some(view) = view else {
        return Err(Fail::new("NotAllowedError", "There is no window to share from."));
    };
    if let Some((picker, delegate)) = LIVE.with(|l| l.borrow_mut().take()) {
        delegate.finish(Err(Fail::new("AbortError", "Share canceled")));
        picker.close();
    }
    let mut req = req;
    let files = write(std::mem::take(&mut req.files)).map_err(|e| Fail::new("NotAllowedError", format!("The share sheet could not be shown ({e})")))?;
    let token = crate::next_token();
    let delegate = Delegate::new(mtm, token, std::mem::take(&mut req.title));
    // SAFETY: the items are strings and URLs, as the picker takes them.
    let picker = unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items(&req, &files)) };
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let bounds = view.bounds();
    let at = NSRect::new(NSPoint::new(bounds.size.width / 2.0, bounds.size.height / 2.0), NSSize::new(1.0, 1.0));
    picker.showRelativeToRect_ofView_preferredEdge(at, &view, NSRectEdge::MinY);
    LIVE.with(|l| *l.borrow_mut() = Some((picker, delegate)));
    Ok(token)
}
