use super::{Callback, Error, Icon, MenuItem, Options};
use std::{
    cell::{Cell, RefCell},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::*,
        System::{
            LibraryLoader::GetModuleHandleW,
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
            Threading::GetCurrentThreadId,
        },
        UI::{
            HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi},
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

const CALLBACK: u32 = WM_APP + 0x61;
const UPDATE: u32 = WM_APP + 0x62;
const STOP: u32 = WM_APP + 0x63;
const ICON_ID: u32 = 1;
const RETRY_TIMER: usize = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);

pub(super) struct Controller {
    stopping: AtomicBool,
    thread_id: AtomicU32,
    window: AtomicUsize,
    revision: AtomicU64,
    pending: Mutex<Option<(u64, Option<Options>)>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Controller {
    pub(super) fn start() -> Result<Arc<Self>, Error> {
        let shared = Arc::new(Self {
            stopping: AtomicBool::new(false),
            thread_id: AtomicU32::new(0),
            window: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
            pending: Mutex::new(None),
            thread: Mutex::new(None),
        });
        let (ready, receive) = mpsc::sync_channel(1);
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("vn-tray".into())
            .spawn(move || {
                if let Err(error) = run(worker.clone(), &ready) {
                    crate::log_later("error", format!("native tray: {error}"));
                    if ready.send(Err(error)).is_err() {
                        crate::log_later("debug", "native tray startup receiver closed".into());
                    }
                }
                worker.stop_admission();
            })
            .map_err(|error| Error::Platform(error.to_string()))?;
        *shared.thread.lock().map_err(|_| Error::Closed)? = Some(thread);
        match receive.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(shared),
            result => {
                shared.stop();
                Err(Error::Platform(match result {
                    Ok(Err(error)) => error,
                    _ => "timed out starting native tray".into(),
                }))
            }
        }
    }
    pub(super) fn set(&self, options: Option<Options>) -> Result<u64, Error> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let mut pending = self.pending.lock().map_err(|_| Error::Closed)?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let revision = self.revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let discarded = pending.replace((revision, options));
        drop(pending);
        drop(discarded);
        let hwnd = HWND(self.window.load(Ordering::Acquire) as *mut _);
        if !hwnd.0.is_null() {
            unsafe {
                if let Err(error) = PostMessageW(Some(hwnd), WM_CANCELMODE, WPARAM(0), LPARAM(0)) {
                    crate::log_later("warn", format!("tray menu cancellation: {error}"));
                }
                if let Err(error) = PostMessageW(Some(hwnd), UPDATE, WPARAM(0), LPARAM(0)) {
                    crate::log_later("warn", format!("tray update wake: {error}"));
                }
            }
        }
        Ok(revision)
    }
    pub(super) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    fn stop_admission(&self) {
        // Serialize the tombstone with set(): an admitted update must never become current after the stop.
        let _pending = self.pending.lock().ok();
        if !self.stopping.swap(true, Ordering::AcqRel) {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }
    pub(super) fn stop(&self) {
        self.stop_admission();
        let hwnd = HWND(self.window.load(Ordering::Acquire) as *mut _);
        unsafe {
            if !hwnd.0.is_null() {
                for message in [WM_CANCELMODE, STOP] {
                    if let Err(error) = PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) {
                        crate::log_later("warn", format!("tray stop wake: {error}"));
                    }
                }
            }
            let thread_id = self.thread_id.load(Ordering::Acquire);
            if thread_id != 0 && thread_id != GetCurrentThreadId() {
                if let Err(error) = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) {
                    crate::log_later("warn", format!("tray thread stop: {error}"));
                }
            }
        }
        let thread = self.thread.lock().ok().and_then(|mut thread| thread.take());
        if let Some(thread) = thread {
            if thread.thread().id() == std::thread::current().id() {
                return;
            }
            let deadline = Instant::now() + Duration::from_secs(1);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() {
                if thread.join().is_err() {
                    crate::log_later("error", "native tray thread panicked".into());
                }
            } else {
                // Explorer can stall inside a shell call: shutdown must not join its worker without a deadline.
                crate::log_later(
                    "warn",
                    "native tray teardown timed out; worker owns cleanup".into(),
                );
            }
        }
    }
}
struct NativeIcon {
    handle: HICON,
    owned: bool,
}
impl Drop for NativeIcon {
    fn drop(&mut self) {
        if self.owned {
            if let Err(error) = unsafe { DestroyIcon(self.handle) } {
                crate::log_later("warn", format!("destroying tray icon: {error}"));
            }
        }
    }
}
struct Snapshot {
    revision: u64,
    options: Options,
    icon: NativeIcon,
}
struct State {
    shared: Arc<Controller>,
    class: Vec<u16>,
    taskbar_created: u32,
    added: Cell<bool>,
    version4: Cell<bool>,
    current: RefCell<Option<std::rc::Rc<Snapshot>>>,
    last_error: RefCell<Option<String>>,
}
impl State {
    fn report(&self, message: String) {
        if self.last_error.borrow().as_ref() != Some(&message) {
            crate::log_later("warn", message.clone());
            *self.last_error.borrow_mut() = Some(message);
        }
    }
    fn data(&self, hwnd: HWND, snapshot: &Snapshot) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: CALLBACK,
            hIcon: snapshot.icon.handle,
            ..Default::default()
        };
        let fallback = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "Application".into());
        let tip = if snapshot.options.tooltip.is_empty() {
            &fallback
        } else {
            &snapshot.options.tooltip
        };
        let mut used = 0;
        for character in tip.chars() {
            let mut units = [0; 2];
            let units = character.encode_utf16(&mut units);
            if used + units.len() >= data.szTip.len() {
                break;
            }
            if let Some(target) = data.szTip.get_mut(used..used + units.len()) {
                target.copy_from_slice(units);
            }
            used += units.len();
        }
        data
    }
    fn remove(&self, hwnd: HWND) {
        if self.added.replace(false) {
            let snapshot = self.current.borrow().clone();
            if let Some(snapshot) = snapshot {
                if !unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data(hwnd, &snapshot)) }.as_bool()
                {
                    self.report("could not remove native tray icon".into());
                }
            }
        }
        self.version4.set(false);
    }
    fn publish(&self, hwnd: HWND) {
        let Some(snapshot) = self.current.borrow().clone() else {
            return;
        };
        let mut data = self.data(hwnd, &snapshot);
        let operation = if self.added.get() {
            NIM_MODIFY
        } else {
            NIM_ADD
        };
        if !unsafe { Shell_NotifyIconW(operation, &data) }.as_bool() {
            self.added.set(false);
            self.report("Explorer did not accept the tray icon; retrying".into());
            return;
        }
        self.added.set(true);
        self.last_error.borrow_mut().take();
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        self.version4
            .set(unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) }.as_bool());
    }
    fn apply(&self, hwnd: HWND, force_icon: bool) {
        let command = self
            .shared
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());
        if let Some((revision, options)) = command {
            let Some(options) = options else {
                self.remove(hwnd);
                self.current.borrow_mut().take();
                return;
            };
            match make_icon(options.icon.as_ref()) {
                Ok(icon) => {
                    let old = self.current.replace(Some(std::rc::Rc::new(Snapshot {
                        revision,
                        options,
                        icon,
                    })));
                    self.publish(hwnd);
                    drop(old);
                }
                Err(error) => self.report(format!("native tray icon: {error}")),
            }
        } else if force_icon {
            let previous = self.current.borrow().clone();
            if let Some(previous) = previous {
                match make_icon(previous.options.icon.as_ref()) {
                    Ok(icon) => {
                        let old = self.current.replace(Some(std::rc::Rc::new(Snapshot {
                            revision: previous.revision,
                            options: previous.options.clone(),
                            icon,
                        })));
                        self.publish(hwnd);
                        drop(old);
                    }
                    Err(error) => self.report(error),
                }
            }
        } else if !self.added.get() {
            self.publish(hwnd);
        }
    }
    fn selected(&self, snapshot: &Snapshot, callback: &Callback) {
        if !self.shared.stopping.load(Ordering::Acquire)
            && snapshot.revision == self.shared.revision()
        {
            super::invoke(callback);
        }
    }
    fn activate(&self, hwnd: HWND, primary: bool, point: Option<POINT>) {
        let Some(snapshot) = self.current.borrow().clone() else {
            return;
        };
        if primary {
            if let Some(callback) = &snapshot.options.on_click {
                self.selected(&snapshot, callback);
                return;
            }
        }
        if snapshot.options.menu.is_empty() {
            return;
        }
        if let Err(error) = self.menu(hwnd, snapshot, point) {
            self.report(format!("native tray menu: {error}"));
        }
    }
    fn menu(
        &self,
        hwnd: HWND,
        snapshot: std::rc::Rc<Snapshot>,
        point: Option<POINT>,
    ) -> Result<(), String> {
        let mut actions = Vec::new();
        let menu = build_menu(
            &snapshot.options.menu,
            &mut actions,
            snapshot.options.on_quit.as_ref(),
        )?;
        let point = point.unwrap_or_else(|| menu_position(hwnd));
        let _foreground_allowed = unsafe { SetForegroundWindow(hwnd) }.as_bool();
        let selected = unsafe {
            TrackPopupMenu(
                menu.0,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                hwnd,
                None,
            )
        }
        .0 as usize;
        if let Err(error) = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) } {
            self.report(format!("tray menu focus wake: {error}"));
        }
        if !unsafe { Shell_NotifyIconW(NIM_SETFOCUS, &self.data(hwnd, &snapshot)) }.as_bool() {
            self.report("could not restore native tray keyboard focus".into());
        }
        if let Some(callback) = selected
            .checked_sub(1)
            .and_then(|index| actions.get(index))
            .and_then(|callback| callback.as_ref())
        {
            self.selected(&snapshot, callback);
        }
        Ok(())
    }
}
struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        if let Err(error) = unsafe { DestroyMenu(self.0) } {
            crate::log_later("warn", format!("destroying tray menu: {error}"));
        }
    }
}
fn build_menu(
    items: &[MenuItem],
    actions: &mut Vec<Option<Callback>>,
    quit: Option<&Callback>,
) -> Result<Menu, String> {
    let menu = Menu(unsafe { CreatePopupMenu() }.map_err(|error| error.to_string())?);
    for item in items {
        match item {
            MenuItem::Separator => unsafe { AppendMenuW(menu.0, MF_SEPARATOR, 0, None) }
                .map_err(|error| error.to_string())?,
            MenuItem::Submenu {
                label,
                disabled,
                items,
            } => {
                let child = build_menu(items, actions, quit)?;
                let label = menu_label(label);
                unsafe {
                    AppendMenuW(
                        menu.0,
                        MF_POPUP | MF_STRING | if *disabled { MF_GRAYED } else { MF_ENABLED },
                        child.0 .0 as usize,
                        PCWSTR(label.as_ptr()),
                    )
                }
                .map_err(|error| error.to_string())?;
                std::mem::forget(child);
            }
            item => {
                let (label, disabled, checked, default, callback) = match item {
                    MenuItem::Action {
                        label,
                        disabled,
                        checked,
                        default,
                        on_select,
                    } => (
                        label.as_str(),
                        *disabled,
                        *checked,
                        *default,
                        on_select.clone(),
                    ),
                    MenuItem::Quit { label, disabled } => (
                        label.as_deref().unwrap_or("Quit"),
                        *disabled,
                        None,
                        false,
                        quit.cloned(),
                    ),
                    _ => continue,
                };
                actions.push(if disabled { None } else { callback });
                let id = actions.len() as u32;
                let label = menu_label(label);
                let flags = MF_STRING
                    | if disabled { MF_GRAYED } else { MF_ENABLED }
                    | if checked == Some(true) {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                unsafe { AppendMenuW(menu.0, flags, id as usize, PCWSTR(label.as_ptr())) }
                    .map_err(|error| error.to_string())?;
                if default {
                    unsafe { SetMenuDefaultItem(menu.0, id, 0) }
                        .map_err(|error| error.to_string())?;
                }
            }
        }
    }
    Ok(menu)
}
fn menu_label(label: &str) -> Vec<u16> {
    label
        .replace('&', "&&")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

fn menu_position(hwnd: HWND) -> POINT {
    let identity = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    if let Ok(rect) = unsafe { Shell_NotifyIconGetRect(&identity) } {
        return POINT {
            x: rect.left + (rect.right - rect.left) / 2,
            y: rect.bottom,
        };
    }
    let mut point = POINT::default();
    if let Err(error) = unsafe { GetCursorPos(&mut point) } {
        crate::log_later("warn", format!("tray menu position: {error}"));
    }
    point
}
fn callback_action(wparam: usize, lparam: isize, version4: bool) -> Option<(bool, Option<POINT>)> {
    let (id, event) = if version4 {
        ((lparam as u32) >> 16, lparam as u32 & 0xffff)
    } else {
        (wparam as u32, lparam as u32)
    };
    if id != ICON_ID {
        return None;
    }
    let primary = match event {
        NIN_SELECT | NIN_KEYSELECT if version4 => true,
        WM_LBUTTONUP if !version4 => true,
        WM_CONTEXTMENU if version4 => false,
        WM_RBUTTONUP if !version4 => false,
        _ => return None,
    };
    let point = if version4 {
        let x = wparam as u16 as i16 as i32;
        let y = (wparam >> 16) as u16 as i16 as i32;
        (x != -1 || y != -1).then_some(POINT { x, y })
    } else {
        None
    };
    Some((primary, point))
}
fn make_icon(source: Option<&Icon>) -> Result<NativeIcon, String> {
    make_icon_foreground(
        source,
        if source.is_some() {
            taskbar_foreground()
        } else {
            0
        },
    )
}

fn make_icon_foreground(source: Option<&Icon>, foreground: u8) -> Result<NativeIcon, String> {
    let Some(source) = source else {
        let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
        let icon = unsafe { LoadIconW(Some(HINSTANCE(module.0)), PCWSTR(1usize as *const u16)) }
            .or_else(|_| unsafe { LoadIconW(None, IDI_APPLICATION) })
            .map_err(|error| error.to_string())?;
        return Ok(NativeIcon {
            handle: icon,
            owned: false,
        });
    };
    let dpi = unsafe { GetDpiForSystem() };
    let side = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.clamp(1, 128) as u32;
    let mut pixels = source.rgba.to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let input = image::RgbaImage::from_raw(source.width, source.height, pixels)
        .ok_or("invalid icon dimensions")?;
    let scale = (side as f64 / source.width as f64).min(side as f64 / source.height as f64);
    let width = ((source.width as f64 * scale).round() as u32).clamp(1, side);
    let height = ((source.height as f64 * scale).round() as u32).clamp(1, side);
    let resized =
        image::imageops::resize(&input, width, height, image::imageops::FilterType::Triangle);
    let mut canvas = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((side - width) / 2),
        i64::from((side - height) / 2),
    );
    let mut pixels = canvas.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        // This BI_RGB-to-HICON boundary takes straight colours: interpolate premultiplied, restore straight only for icon creation.
        if source.template {
            for channel in &mut pixel[..3] {
                *channel = foreground;
            }
        } else if alpha != 0 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: side as i32,
            biHeight: -(side as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let color = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
        .map_err(|error| error.to_string())?;
    struct Bitmap(HBITMAP);
    impl Drop for Bitmap {
        fn drop(&mut self) {
            if !unsafe { DeleteObject(HGDIOBJ(self.0 .0)) }.as_bool() {
                crate::log_later("warn", "destroying tray bitmap failed".into());
            }
        }
    }
    let color = Bitmap(color);
    if bits.is_null() {
        return Err("tray bitmap has no pixel buffer".into());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
    }
    let mask = Bitmap(unsafe { CreateBitmap(side as i32, side as i32, 1, 1, None) });
    if mask.0 .0.is_null() {
        return Err("could not create tray icon mask".into());
    }
    let handle = unsafe {
        CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask.0,
            hbmColor: color.0,
            ..Default::default()
        })
    }
    .map_err(|error| error.to_string())?;
    Ok(NativeIcon {
        handle,
        owned: true,
    })
}
fn taskbar_foreground() -> u8 {
    let mut light = 1u32;
    let mut length = 4;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut light as *mut u32).cast()),
            Some(&mut length),
        )
    };
    if result.is_ok() && light == 0 {
        255
    } else {
        0
    }
}
fn run(
    shared: Arc<Controller>,
    ready: &mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    shared
        .thread_id
        .store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    struct ThreadIdentity(Arc<Controller>);
    impl Drop for ThreadIdentity {
        fn drop(&mut self) {
            self.0.thread_id.store(0, Ordering::Release);
            self.0.window.store(0, Ordering::Release);
        }
    }
    let _identity = ThreadIdentity(shared.clone());
    let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE(module.0);
    let class: Vec<u16> = format!(
        "VueNative.Tray.{}.{}",
        std::process::id(),
        NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let descriptor = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&descriptor) } == 0 {
        return Err("could not register native tray window".into());
    }
    struct Class(HINSTANCE, Vec<u16>);
    impl Drop for Class {
        fn drop(&mut self) {
            if let Err(error) = unsafe { UnregisterClassW(PCWSTR(self.1.as_ptr()), Some(self.0)) } {
                crate::log_later("warn", format!("unregistering tray window: {error}"));
            }
        }
    }
    let _class = Class(instance, class.clone());
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    if taskbar_created == 0 {
        return Err("could not register tray recovery message".into());
    }
    let state = Box::new(State {
        shared: shared.clone(),
        class,
        taskbar_created,
        added: Cell::new(false),
        version4: Cell::new(false),
        current: RefCell::new(None),
        last_error: RefCell::new(None),
    });
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(state.class.as_ptr()),
            w!("vue-native tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            Some(&*state as *const State as *const _),
        )
    }
    .map_err(|error| error.to_string())?;
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            if unsafe { IsWindow(Some(self.0)) }.as_bool() {
                if let Err(error) = unsafe { DestroyWindow(self.0) } {
                    crate::log_later("warn", format!("destroying tray window: {error}"));
                }
            }
        }
    }
    let _window = Window(hwnd);
    shared.window.store(hwnd.0 as usize, Ordering::Release);
    if unsafe { SetTimer(Some(hwnd), RETRY_TIMER, 1000, None) } == 0 {
        return Err("could not create tray recovery timer".into());
    }
    if shared.stopping.load(Ordering::Acquire) {
        return Err("native tray startup cancelled".into());
    }
    if ready.send(Ok(())).is_err() {
        return Err("native tray startup was abandoned".into());
    }
    let mut message = MSG::default();
    while !shared.stopping.load(Ordering::Acquire) {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if result < 0 {
            return Err("native tray message loop failed".into());
        }
        if result == 0 {
            break;
        }
        let _translated = unsafe { TranslateMessage(&message) }.as_bool();
        unsafe {
            DispatchMessageW(&message);
        }
    }
    state.remove(hwnd);
    Ok(())
}
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        dispatch(hwnd, message, wparam, lparam)
    }));
    match result {
        Ok(result) => result,
        Err(_) => {
            crate::log_later("error", "native tray message panicked".into());
            LRESULT(0)
        }
    }
}
unsafe fn dispatch(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const State;
    if !pointer.is_null() {
        let state = unsafe { &*pointer };
        match message {
            CALLBACK => {
                if let Some((primary, point)) =
                    callback_action(wparam.0, lparam.0, state.version4.get())
                {
                    state.activate(hwnd, primary, point);
                }
                return LRESULT(0);
            }
            UPDATE => {
                state.apply(hwnd, false);
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == RETRY_TIMER => {
                state.apply(hwnd, false);
                return LRESULT(0);
            }
            WM_THEMECHANGED | WM_SETTINGCHANGE | WM_DPICHANGED => {
                state.apply(hwnd, true);
                return LRESULT(0);
            }
            STOP | WM_CLOSE => {
                if let Err(error) = unsafe { DestroyWindow(hwnd) } {
                    state.report(format!("closing tray window: {error}"));
                }
                return LRESULT(0);
            }
            WM_DESTROY => {
                state.remove(hwnd);
                unsafe {
                    if let Err(error) = KillTimer(Some(hwnd), RETRY_TIMER) {
                        state.report(format!("stopping tray recovery timer: {error}"));
                    }
                    PostQuitMessage(0);
                }
                return LRESULT(0);
            }
            WM_ENDSESSION if wparam.0 != 0 => {
                let snapshot = state.current.borrow().clone();
                if let Some(snapshot) = snapshot {
                    if let Some(callback) = &snapshot.options.on_quit {
                        state.selected(&snapshot, callback);
                    }
                }
            }
            WM_NCDESTROY => unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            },
            _ if message == state.taskbar_created => {
                state.added.set(false);
                state.version4.set(false);
                state.publish(hwnd);
                return LRESULT(0);
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alpha_icons_draw_premultiplied_colours_and_template_edges() {
        struct Surface {
            dc: HDC,
            bitmap: HBITMAP,
            previous: HGDIOBJ,
            bits: *mut u8,
        }
        impl Drop for Surface {
            fn drop(&mut self) {
                unsafe {
                    SelectObject(self.dc, self.previous);
                    let _bitmap_deleted = DeleteObject(HGDIOBJ(self.bitmap.0)).as_bool();
                    let _dc_deleted = DeleteDC(self.dc).as_bool();
                }
            }
        }
        let dc = unsafe { CreateCompatibleDC(None) };
        assert!(!dc.0.is_null());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 32,
                biHeight: -32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
                .unwrap();
        assert!(!bits.is_null());
        let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        let surface = Surface {
            dc,
            bitmap,
            previous,
            bits: bits.cast(),
        };
        for template in [false, true] {
            let mut source = Icon::rgba(1, 1, vec![255, 0, 0, 128]).unwrap();
            source.template = template;
            let icon = make_icon_foreground(Some(&source), 255).unwrap();
            unsafe {
                std::ptr::write_bytes(surface.bits, 0, 32 * 32 * 4);
            }
            unsafe { DrawIconEx(surface.dc, 0, 0, icon.handle, 32, 32, 0, None, DI_NORMAL) }
                .unwrap();
            assert!(unsafe { GdiFlush() }.as_bool());
            let pixel =
                unsafe { std::slice::from_raw_parts(surface.bits.add((16 * 32 + 16) * 4), 4) };
            assert!(
                (126..=130).contains(&pixel[2]),
                "red/foreground must blend to half intensity: {pixel:?}"
            );
            if template {
                assert!(
                    (126..=130).contains(&pixel[0]) && (126..=130).contains(&pixel[1]),
                    "template edges must blend equally: {pixel:?}"
                );
            } else {
                assert!(
                    pixel[0] <= 1 && pixel[1] <= 1,
                    "coloured icon channels: {pixel:?}"
                );
            }
        }
    }
    #[test]
    fn menu_labels_preserve_literal_ampersands_and_unicode() {
        assert_eq!(
            String::from_utf16(&menu_label("A&B 😀")[..7]).unwrap(),
            "A&&B 😀"
        );
    }
    #[test]
    fn v4_callback_decodes_id_and_signed_icon_coordinates() {
        let (primary, point) = callback_action(
            0xff80_ff90,
            ((ICON_ID << 16) | NIN_KEYSELECT) as isize,
            true,
        )
        .unwrap();
        assert!(primary);
        assert_eq!(point.map(|p| (p.x, p.y)), Some((-112, -128)));
        assert!(callback_action(0, ((2 << 16) | NIN_SELECT) as isize, true).is_none());
        assert_eq!(
            callback_action(1, WM_RBUTTONUP as isize, false).map(|v| v.0),
            Some(false)
        );
        assert!(callback_action(1, WM_MOUSEMOVE as isize, false).is_none());
        assert!(
            callback_action(0, ((ICON_ID << 16) | WM_LBUTTONUP) as isize, true).is_none(),
            "v4 must not double-deliver mouse up and NIN_SELECT"
        );
    }
}

#[cfg(test)]
#[path = "windows_native_probe.rs"]
mod native_probe;
