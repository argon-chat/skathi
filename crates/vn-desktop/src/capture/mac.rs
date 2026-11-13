//! ScreenCaptureKit sources (macOS 13+) on the pool worker. SCK answers on its own queues into channels the
//! worker waits on with a deadline; `NSScreen` is read on the main queue. Thumbnails use
//! `SCScreenshotManager` on macOS 14+, else `CGWindowListCreateImage`.

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::ptr::NonNull;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::encode::{Encoding, RefEncode};
use objc2::msg_send;
use objc2::rc::{autoreleasepool, Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::NSString;
use serde_json::Value;

use super::listing::{self, DisplayRow, Rect, WindowRow};
use super::{fit, png_url, Options};
use crate::Fail;

#[repr(C)]
struct CGImage {
    _private: [u8; 0],
}

unsafe impl RefEncode for CGImage {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

#[link(name = "ScreenCaptureKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
    fn CGImageRetain(image: *mut CGImage) -> *mut CGImage;
    fn CGImageRelease(image: *mut CGImage);
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextSetInterpolationQuality(context: *mut c_void, quality: i32);
    fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: *mut CGImage);
    fn CGContextRelease(context: *mut c_void);
    static kCGColorSpaceSRGB: *const c_void;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
const PREMULTIPLIED_RGBA: u32 = 1 | (4 << 12);
const INTERPOLATION_HIGH: i32 = 3;
const LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
const LIST_INCLUDING_WINDOW: u32 = 1 << 3;
const IMAGE_IGNORE_FRAMING: u32 = 1 << 0;
const IMAGE_NOMINAL_RESOLUTION: u32 = 1 << 4;
const USER_DECLINED: isize = -3801;
const PREVIEW_DEADLINE: Duration = Duration::from_secs(2);

struct CgImage(NonNull<CGImage>);
// SAFETY: CGImage is immutable and its retain count is thread-safe.
unsafe impl Send for CgImage {}

impl CgImage {
    unsafe fn retain(image: *mut CGImage) -> Option<Self> {
        NonNull::new(image).map(|image| {
            unsafe { CGImageRetain(image.as_ptr()) };
            CgImage(image)
        })
    }
    unsafe fn adopt(image: *mut CGImage) -> Option<Self> {
        NonNull::new(image).map(CgImage)
    }
}

impl Drop for CgImage {
    fn drop(&mut self) {
        unsafe { CGImageRelease(self.0.as_ptr()) }
    }
}

struct Sent(Retained<AnyObject>);
// SAFETY: only immutable ScreenCaptureKit results (SCShareableContent) cross.
unsafe impl Send for Sent {}

fn class(name: &CStr) -> Result<&'static AnyClass, Fail> {
    AnyClass::get(name).ok_or_else(|| {
        Fail::new("NotSupportedError", format!("{} is unavailable", name.to_string_lossy()))
    })
}

unsafe fn error_info(error: *mut AnyObject) -> (isize, String) {
    let Some(error) = (unsafe { error.as_ref() }) else {
        return (0, "unknown error".into());
    };
    let code: isize = unsafe { msg_send![error, code] };
    let text: Option<Retained<NSString>> = unsafe { msg_send![error, localizedDescription] };
    (code, text.map_or_else(|| "unknown error".into(), |t| t.to_string()))
}

fn sck_failure(code: isize, text: &str) -> Fail {
    if code == USER_DECLINED {
        let (name, message) = vn_permissions::display_capture_refusal();
        return Fail::new(name, message);
    }
    Fail::new("OperationError", format!("ScreenCaptureKit: {text} ({code})"))
}

unsafe fn objects(array: &AnyObject) -> Vec<Retained<AnyObject>> {
    let count: usize = unsafe { msg_send![array, count] };
    (0..count)
        .filter_map(|i| -> Option<Retained<AnyObject>> { unsafe { msg_send![array, objectAtIndex: i] } })
        .collect()
}

fn text(value: Option<Retained<NSString>>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn rect(frame: CGRect) -> Rect {
    Rect { x: frame.origin.x, y: frame.origin.y, width: frame.size.width, height: frame.size.height }
}

fn cg_rect(rect: Rect) -> CGRect {
    CGRect::new(CGPoint::new(rect.x, rect.y), CGSize::new(rect.width, rect.height))
}

pub(super) fn sources(options: Options) -> Result<Vec<Value>, Fail> {
    autoreleasepool(|_| unsafe { list(options) })
}

unsafe fn list(options: Options) -> Result<Vec<Value>, Fail> {
    let content = shareable_content()?;
    let main = unsafe { CGMainDisplayID() };
    let mut displays = Vec::new();
    let mut display_objects = HashMap::new();
    let all: Retained<AnyObject> = unsafe { msg_send![&*content, displays] };
    for display in unsafe { objects(&all) } {
        let id: u32 = unsafe { msg_send![&*display, displayID] };
        let frame = rect(unsafe { msg_send![&*display, frame] });
        displays.push(DisplayRow {
            id,
            frame,
            scale: unsafe { pixel_scale(id, frame.width) },
            primary: id == main,
            work_area: None,
            name: None,
        });
        display_objects.insert(id, display);
    }
    listing::order_displays(&mut displays);
    let mut windows = Vec::new();
    if options.windows {
        let own = std::process::id() as i32;
        let all: Retained<AnyObject> = unsafe { msg_send![&*content, windows] };
        for window in unsafe { objects(&all) } {
            let row = unsafe { window_row(&window) };
            if listing::keep_window(&row, own) && windows.len() < listing::MAX_WINDOWS {
                windows.push((row, window));
            }
        }
    }
    if options.screens {
        screen_details(&mut displays);
    }

    let modern = AnyClass::get(c"SCScreenshotManager").is_some();
    let size = |frame: &Rect| {
        fit(frame.width.round() as u32, frame.height.round() as u32, options.thumbnail_width, options.thumbnail_height)
    };
    let mut screen_shots = Vec::new();
    if options.screens {
        for display in &displays {
            let target = display_objects.get(&display.id).map(|d| Target::Display(d, display.frame));
            let size = size(&display.frame);
            screen_shots.push((unsafe { start_preview(target, size, modern) }, size));
        }
    }
    let mut window_shots = Vec::new();
    for (row, window) in &windows {
        let size = size(&row.frame);
        window_shots.push((unsafe { start_preview(Some(Target::Window(window, row.id)), size, modern) }, size));
    }
    let deadline = Instant::now() + PREVIEW_DEADLINE;

    let mut result = Vec::new();
    if options.screens {
        for (index, (display, (shot, size))) in displays.iter().zip(screen_shots).enumerate() {
            result.push(listing::screen_json(index, display, finish_preview(shot, size, deadline)));
        }
    }
    let mut icons: HashMap<i32, Option<String>> = HashMap::new();
    for ((row, _), (shot, size)) in windows.iter().zip(window_shots) {
        let thumbnail = finish_preview(shot, size, deadline);
        let icon = match row.pid {
            Some(pid) if options.with_icons => {
                icons.entry(pid).or_insert_with(|| unsafe { app_icon(pid) }).clone()
            }
            _ => None,
        };
        let display = listing::display_for(&row.frame, &displays);
        result.push(listing::window_json(row, display, thumbnail, icon));
    }
    Ok(result)
}

fn shareable_content() -> Result<Retained<AnyObject>, Fail> {
    let cls = class(c"SCShareableContent")?;
    let (send, receive) = mpsc::sync_channel::<Result<Sent, (isize, String)>>(1);
    let handler = RcBlock::new(move |content: *mut AnyObject, error: *mut AnyObject| {
        let result = match unsafe { Retained::retain(content) } {
            Some(content) => Ok(Sent(content)),
            None => Err(unsafe { error_info(error) }),
        };
        let _ = send.try_send(result);
    });
    unsafe {
        let _: () = msg_send![
            cls,
            getShareableContentExcludingDesktopWindows: Bool::YES,
            onScreenWindowsOnly: Bool::YES,
            completionHandler: &*handler
        ];
    }
    match receive.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(content)) => Ok(content.0),
        Ok(Err((code, text))) => Err(sck_failure(code, &text)),
        Err(_) => Err(Fail::new("TimeoutError", "ScreenCaptureKit did not list the shareable content within 10s")),
    }
}

unsafe fn pixel_scale(display: u32, width: f64) -> f64 {
    let mode = unsafe { CGDisplayCopyDisplayMode(display) };
    if mode.is_null() || width <= 0.0 {
        return 1.0;
    }
    let pixels = unsafe { CGDisplayModeGetPixelWidth(mode) };
    unsafe { CGDisplayModeRelease(mode) };
    if pixels == 0 { 1.0 } else { pixels as f64 / width }
}

unsafe fn window_row(window: &AnyObject) -> WindowRow {
    let frame: CGRect = unsafe { msg_send![window, frame] };
    let on_screen: Bool = unsafe { msg_send![window, isOnScreen] };
    let app: Option<Retained<AnyObject>> = unsafe { msg_send![window, owningApplication] };
    let mut row = WindowRow {
        id: unsafe { msg_send![window, windowID] },
        title: unsafe { text(msg_send![window, title]) },
        frame: rect(frame),
        layer: unsafe { msg_send![window, windowLayer] },
        on_screen: on_screen.as_bool(),
        ..WindowRow::default()
    };
    if let Some(app) = app {
        let pid: i32 = unsafe { msg_send![&*app, processID] };
        row.pid = Some(pid);
        row.bundle_id = unsafe { text(msg_send![&*app, bundleIdentifier]) };
        row.app_name = unsafe { text(msg_send![&*app, applicationName]) };
    }
    row
}

/// Each display's work area and name, from `NSScreen` on the main queue; if it does not answer within
/// half a second the whole display is the work area.
fn screen_details(displays: &mut [DisplayRow]) {
    let (send, receive) = mpsc::sync_channel(1);
    dispatch2::DispatchQueue::main().exec_async(move || {
        let screens = std::panic::catch_unwind(|| autoreleasepool(|_| unsafe { read_screens() }));
        let _ = send.try_send(screens.unwrap_or_default());
    });
    let Ok(screens) = receive.recv_timeout(Duration::from_millis(500)) else {
        return;
    };
    for display in displays {
        if let Some((_, work_area, name)) = screens.iter().find(|(id, ..)| *id == display.id) {
            display.work_area = Some(*work_area);
            display.name = name.clone();
        }
    }
}

unsafe fn read_screens() -> Vec<(u32, Rect, Option<String>)> {
    let Some(cls) = AnyClass::get(c"NSScreen") else {
        return Vec::new();
    };
    let screens: Option<Retained<AnyObject>> = unsafe { msg_send![cls, screens] };
    let Some(screens) = screens else {
        return Vec::new();
    };
    let screens = unsafe { objects(&screens) };
    let Some(primary) = screens.first() else {
        return Vec::new();
    };
    let primary: CGRect = unsafe { msg_send![&**primary, frame] };
    let key = NSString::from_str("NSScreenNumber");
    let mut out = Vec::new();
    for screen in &screens {
        let description: Option<Retained<AnyObject>> = unsafe { msg_send![&**screen, deviceDescription] };
        let number: Option<Retained<AnyObject>> =
            description.and_then(|d| unsafe { msg_send![&*d, objectForKey: &*key] });
        let Some(number) = number else { continue };
        let id: u32 = unsafe { msg_send![&*number, unsignedIntValue] };
        let visible: CGRect = unsafe { msg_send![&**screen, visibleFrame] };
        let name: Option<Retained<NSString>> = unsafe { msg_send![&**screen, localizedName] };
        out.push((id, listing::flip(rect(visible), primary.size.height), name.map(|n| n.to_string())));
    }
    out
}

enum Target<'a> {
    Display(&'a Retained<AnyObject>, Rect),
    Window(&'a Retained<AnyObject>, u32),
}

type Shot = Result<CgImage, (isize, String)>;

enum Preview {
    None,
    Ready(Result<CgImage, Fail>),
    Waiting(mpsc::Receiver<Shot>),
}

unsafe fn start_preview(target: Option<Target<'_>>, (width, height): (u32, u32), modern: bool) -> Preview {
    let Some(target) = target else {
        return Preview::Ready(Err(Fail::new("NotFoundError", "the display is no longer available")));
    };
    if width == 0 || height == 0 {
        return Preview::None;
    }
    let result = if modern {
        unsafe { screenshot(&target, width, height) }.map(Preview::Waiting)
    } else {
        unsafe { window_list_image(&target) }.map(|image| Preview::Ready(Ok(image)))
    };
    result.unwrap_or_else(|error| Preview::Ready(Err(error)))
}

fn finish_preview(preview: Preview, (width, height): (u32, u32), deadline: Instant) -> Result<String, Fail> {
    let image = match preview {
        Preview::None => return Ok(String::new()),
        Preview::Ready(image) => image?,
        Preview::Waiting(receive) => receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| Fail::new("TimeoutError", "the source did not supply a preview within 2s"))?
            .map_err(|(code, text)| sck_failure(code, &text))?,
    };
    let mut rgba = render(&image, width, height)?;
    listing::opaque(&mut rgba);
    png_url(&rgba, width, height)
}

unsafe fn content_filter(target: &Target<'_>) -> Result<Retained<AnyObject>, Fail> {
    let filter: Allocated<AnyObject> = unsafe { msg_send![class(c"SCContentFilter")?, alloc] };
    let filter: Option<Retained<AnyObject>> = match target {
        Target::Display(display, _) => {
            let none: Retained<AnyObject> = unsafe { msg_send![class(c"NSArray")?, array] };
            unsafe { msg_send![filter, initWithDisplay: &***display, excludingWindows: &*none] }
        }
        Target::Window(window, _) => unsafe { msg_send![filter, initWithDesktopIndependentWindow: &***window] },
    };
    filter.ok_or_else(|| Fail::new("OperationError", "ScreenCaptureKit refused the content filter"))
}

unsafe fn screenshot(
    target: &Target<'_>,
    width: u32,
    height: u32,
) -> Result<mpsc::Receiver<Shot>, Fail> {
    let manager = class(c"SCScreenshotManager")?;
    let filter = unsafe { content_filter(target)? };
    let config: Retained<AnyObject> = unsafe { msg_send![class(c"SCStreamConfiguration")?, new] };
    unsafe {
        let _: () = msg_send![&*config, setWidth: width as usize];
        let _: () = msg_send![&*config, setHeight: height as usize];
        let _: () = msg_send![&*config, setShowsCursor: Bool::NO];
    }
    let (send, receive) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |image: *mut CGImage, error: *mut AnyObject| {
        let result = unsafe { CgImage::retain(image) }.ok_or_else(|| unsafe { error_info(error) });
        let _ = send.try_send(result);
    });
    unsafe {
        let _: () = msg_send![
            manager,
            captureImageWithFilter: &*filter,
            configuration: &*config,
            completionHandler: &*handler
        ];
    }
    Ok(receive)
}

type WindowListCreateImage = unsafe extern "C" fn(CGRect, u32, u32, u32) -> *mut CGImage;

/// macOS 13: `CGWindowListCreateImage`, looked up rather than linked (the macOS 15 SDK marks it obsoleted).
unsafe fn window_list_image(target: &Target<'_>) -> Result<CgImage, Fail> {
    let symbol = unsafe { dlsym(RTLD_DEFAULT, c"CGWindowListCreateImage".as_ptr()) };
    if symbol.is_null() {
        return Err(Fail::new("NotSupportedError", "CGWindowListCreateImage is unavailable"));
    }
    // SAFETY: the C prototype is `CGImageRef (CGRect, CGWindowListOption,
    // CGWindowID, CGWindowImageOption)`, both options `uint32_t`.
    let create: WindowListCreateImage = unsafe { std::mem::transmute(symbol) };
    let image = match target {
        Target::Display(_, frame) => unsafe {
            create(cg_rect(*frame), LIST_ON_SCREEN_ONLY, 0, IMAGE_NOMINAL_RESOLUTION)
        },
        Target::Window(_, id) => {
            let null = CGRect::new(CGPoint::new(f64::INFINITY, f64::INFINITY), CGSize::new(0.0, 0.0));
            unsafe { create(null, LIST_INCLUDING_WINDOW, *id, IMAGE_IGNORE_FRAMING | IMAGE_NOMINAL_RESOLUTION) }
        }
    };
    unsafe { CgImage::adopt(image) }
        .ok_or_else(|| Fail::new("OperationError", "the window server returned no image"))
}

fn render(image: &CgImage, width: u32, height: u32) -> Result<Vec<u8>, Fail> {
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    unsafe {
        let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        if space.is_null() {
            return Err(Fail::new("OperationError", "CGColorSpaceCreateWithName failed"));
        }
        let context = CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            width as usize * 4,
            space,
            PREMULTIPLIED_RGBA,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return Err(Fail::new("OperationError", "CGBitmapContextCreate failed"));
        }
        CGContextSetInterpolationQuality(context, INTERPOLATION_HIGH);
        CGContextDrawImage(context, cg_rect(Rect { width: width as f64, height: height as f64, ..Rect::default() }), image.0.as_ptr());
        CGContextRelease(context);
    }
    Ok(rgba)
}

unsafe fn app_icon(pid: i32) -> Option<String> {
    let cls = AnyClass::get(c"NSRunningApplication")?;
    let app: Option<Retained<AnyObject>> = unsafe { msg_send![cls, runningApplicationWithProcessIdentifier: pid] };
    let icon: Option<Retained<AnyObject>> = unsafe { msg_send![&*app?, icon] };
    let mut proposed = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(32.0, 32.0));
    let image: *mut CGImage = unsafe {
        msg_send![
            &*icon?,
            CGImageForProposedRect: &mut proposed as *mut CGRect,
            context: None::<&AnyObject>,
            hints: None::<&AnyObject>
        ]
    };
    let image = unsafe { CgImage::retain(image) }?;
    let mut rgba = render(&image, 32, 32).ok()?;
    listing::unpremultiply(&mut rgba);
    png_url(&rgba, 32, 32).ok()
}
