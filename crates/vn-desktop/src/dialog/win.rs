//! `IFileOpenDialog` / `IFileSaveDialog` on a `vn-dialog` thread (mod.rs).
//!
//! Threading follows gpui's `dialog.rs` (Apache-2.0, zed-industries/zed): a dedicated STA, a
//! 100 ms thread timer posting `WM_CLOSE` to the thread's `#32770` windows once the owner is
//! closing, and the owner kept alive until the loop exits. Unlike gpui this sets filters, a client
//! GUID and `FOS_NOCHANGEDIR`, and answers through a callback.

use std::cell::RefCell;
use std::path::PathBuf;

use windows::core::{s, w, Interface, BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::{
    TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TDF_USE_HICON_MAIN,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassLongPtrW, GCLP_HICON, HICON};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
    SHGetKnownFolderItem, SHGetKnownFolderPath, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads,
    FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST,
    FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, KF_FLAG_DEFAULT,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetClassNameW, KillTimer, PostMessageW, SetTimer, WM_CLOSE};

use super::{Cancellation, Kind, Known, Native, Start, ALL_FILES};
use crate::Fail;

fn known_id(k: Known) -> windows::core::GUID {
    match k {
        Known::Desktop => FOLDERID_Desktop,
        Known::Documents => FOLDERID_Documents,
        Known::Downloads => FOLDERID_Downloads,
        Known::Music => FOLDERID_Music,
        Known::Pictures => FOLDERID_Pictures,
        Known::Videos => FOLDERID_Videos,
    }
}

pub(super) fn known_folder(k: Known) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&known_id(k), KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn fail(what: &str, e: &windows::core::Error) -> Fail {
    Fail::new("InvalidStateError", format!("the file dialog failed ({what}): {}", e.message())).code("EIO").os(e.code().0)
}

struct Apartment;

impl Apartment {
    fn new() -> Result<Self, Fail> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .map_err(|e| fail("CoInitializeEx", &e))?;
        Ok(Apartment)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

thread_local! {
    static CANCELLATION: RefCell<Option<Cancellation>> = const { RefCell::new(None) };
}

/// Closes this thread's dialog once its owner is closing; a thread timer, dispatched by the modal loop.
struct CancellationTimer(usize);

impl CancellationTimer {
    fn new(c: Cancellation) -> Result<Self, Fail> {
        let timer = unsafe { SetTimer(None, 0, 100, Some(cancel_dialog)) };
        if timer == 0 {
            return Err(fail("SetTimer", &windows::core::Error::from_thread()));
        }
        CANCELLATION.with(|slot| slot.replace(Some(c)));
        Ok(CancellationTimer(timer))
    }
}

impl Drop for CancellationTimer {
    fn drop(&mut self) {
        CANCELLATION.with(|slot| slot.take());
        unsafe {
            let _ = KillTimer(None, self.0);
        }
    }
}

unsafe extern "system" fn cancel_dialog(_: HWND, _: u32, _: usize, _: u32) {
    let requested = CANCELLATION.with(|slot| slot.borrow().as_ref().is_some_and(Cancellation::is_requested));
    if requested {
        unsafe {
            let _ = EnumThreadWindows(GetCurrentThreadId(), Some(close_dialog), LPARAM(0));
        }
    }
}

/// Shell dialogs can open further modal dialogs (an overwrite prompt): close only dialog windows
/// on this thread, never the hidden COM windows beside them.
unsafe extern "system" fn close_dialog(hwnd: HWND, _: LPARAM) -> BOOL {
    let mut class_name = [0u16; 32];
    let length = unsafe { GetClassNameW(hwnd, &mut class_name) } as usize;
    if class_name.get(..length) == Some(unsafe { w!("#32770").as_wide() }) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    TRUE
}

/// `TaskDialogIndirect`, resolved at run time: it exists only in comctl32 v6 (via manifest), and
/// linking it statically would stop manifest-less binaries, unit tests included, from loading.
type TaskDialogIndirectFn = unsafe extern "system" fn(
    *const TASKDIALOGCONFIG,
    *mut i32,
    *mut i32,
    *mut BOOL,
) -> windows::core::HRESULT;

fn task_dialog_indirect() -> Option<TaskDialogIndirectFn> {
    static F: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let p = *F.get_or_init(|| unsafe {
        let module = LoadLibraryW(w!("comctl32.dll")).ok()?;
        GetProcAddress(module, s!("TaskDialogIndirect")).map(|f| f as usize)
    });
    // SAFETY: the export's documented signature.
    p.map(|p| unsafe { std::mem::transmute::<usize, TaskDialogIndirectFn>(p) })
}

/// The first button's id; button `i` is `BUTTON_BASE + i`.
const BUTTON_BASE: i32 = 100;

/// Asks `q` in a TaskDialog owned by `owner` (0 = none): the button index, or `Ok(None)` for Esc,
/// the close box, or the owner closing (the same 100 ms timer; a TaskDialog is a `#32770` too).
pub(super) fn ask(q: &super::Ask, owner: isize, cancel: &Cancellation) -> Result<Option<usize>, Fail> {
    let Some(show) = task_dialog_indirect() else {
        return Err(Fail::new(
            "NotSupportedError",
            "TaskDialogIndirect is not available: this process has no Common Controls 6 manifest",
        ));
    };
    let _timer = CancellationTimer::new(cancel.clone())?;
    if cancel.is_requested() {
        return Ok(None);
    }
    let title = HSTRING::from(q.title.as_str());
    let instruction = HSTRING::from(q.instruction.as_str());
    let content = HSTRING::from(q.content.as_str());
    let labels: Vec<HSTRING> = q.buttons.iter().map(|b| HSTRING::from(b.as_str())).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| TASKDIALOG_BUTTON { nButtonID: BUTTON_BASE + i as i32, pszButtonText: PCWSTR(l.as_ptr()) })
        .collect();
    let mut config = TASKDIALOGCONFIG::default();
    config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
    config.hwndParent = HWND(owner as *mut core::ffi::c_void);
    let mut flags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW;
    let icon = if owner != 0 { unsafe { GetClassLongPtrW(HWND(owner as *mut core::ffi::c_void), GCLP_HICON) } } else { 0 };
    if icon != 0 {
        flags |= TDF_USE_HICON_MAIN;
        config.Anonymous1.hMainIcon = HICON(icon as *mut core::ffi::c_void);
    }
    // TASKDIALOGCONFIG is packed: its fields are assigned whole, never borrowed.
    config.dwFlags = flags;
    config.pszWindowTitle = PCWSTR(title.as_ptr());
    config.pszMainInstruction = PCWSTR(instruction.as_ptr());
    if !q.content.is_empty() {
        config.pszContent = PCWSTR(content.as_ptr());
    }
    config.cButtons = buttons.len() as u32;
    config.pButtons = buttons.as_ptr();
    config.nDefaultButton = BUTTON_BASE + q.default.min(buttons.len().saturating_sub(1)) as i32;
    let mut pressed = 0i32;
    let hr = unsafe { show(&config, &mut pressed, std::ptr::null_mut(), std::ptr::null_mut()) };
    if let Err(e) = hr.ok() {
        return Err(fail("TaskDialogIndirect", &e));
    }
    if cancel.is_requested() {
        return Ok(None);
    }
    let i = pressed - BUTTON_BASE;
    Ok((0..buttons.len() as i32).contains(&i).then_some(i as usize))
}

/// Shows the dialog for `n`; `Ok(None)` is a dismissal, including the owner closing.
pub(super) fn pick(n: &Native, owner: isize, cancel: &Cancellation) -> Result<Option<Vec<PathBuf>>, Fail> {
    let _apartment = Apartment::new()?;
    let _timer = CancellationTimer::new(cancel.clone())?;
    if cancel.is_requested() {
        return Ok(None);
    }
    let result = unsafe { show(n, owner) };
    if cancel.is_requested() {
        return Ok(None);
    }
    result
}

fn item_at(path: &std::path::Path) -> Option<IShellItem> {
    // The shell's parser does not take the long-path prefix.
    let s = path.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(s), None).ok() }
}

fn path_of(item: &IShellItem) -> Result<PathBuf, Fail> {
    unsafe {
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| fail("GetDisplayName", &e))?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from).map_err(|_| Fail::new("InvalidStateError", "a picked path is not valid UTF-16").code("EIO"))
    }
}

unsafe fn show(n: &Native, owner: isize) -> Result<Option<Vec<PathBuf>>, Fail> {
    let dialog: IFileDialog = unsafe {
        match n.kind {
            Kind::Save => CoCreateInstance::<_, IFileDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER),
            Kind::Open | Kind::Folder => CoCreateInstance::<_, IFileDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER),
        }
    }
    .map_err(|e| fail("CoCreateInstance", &e))?;

    let mut opts = unsafe { dialog.GetOptions() }.map_err(|e| fail("GetOptions", &e))?;
    // Without FOS_NOCHANGEDIR the dialog moves the process's current directory.
    opts |= FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR;
    match n.kind {
        Kind::Open => {
            opts |= FOS_FILEMUSTEXIST;
            if n.multiple {
                opts |= FOS_ALLOWMULTISELECT;
            }
        }
        Kind::Folder => opts |= FOS_PICKFOLDERS,
        Kind::Save => opts |= FOS_OVERWRITEPROMPT,
    }
    unsafe { dialog.SetOptions(opts) }.map_err(|e| fail("SetOptions", &e))?;

    let mut names: Vec<HSTRING> = Vec::new();
    let mut specs: Vec<HSTRING> = Vec::new();
    if n.kind != Kind::Folder {
        for f in &n.filters {
            names.push(HSTRING::from(f.name.as_str()));
            specs.push(HSTRING::from(f.spec()));
        }
        if n.allow_all {
            names.push(HSTRING::from(ALL_FILES));
            specs.push(HSTRING::from("*.*"));
        }
    }
    if !names.is_empty() {
        let filter: Vec<COMDLG_FILTERSPEC> = names
            .iter()
            .zip(&specs)
            .map(|(name, spec)| COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) })
            .collect();
        unsafe { dialog.SetFileTypes(&filter) }.map_err(|e| fail("SetFileTypes", &e))?;
        unsafe { dialog.SetFileTypeIndex(1) }.map_err(|e| fail("SetFileTypeIndex", &e))?;
    }

    let guid = windows::core::GUID::from_u128(n.client);
    unsafe { dialog.SetClientGuid(&guid) }.map_err(|e| fail("SetClientGuid", &e))?;

    let start = match &n.start {
        Some(Start::Known(k)) => unsafe { SHGetKnownFolderItem::<IShellItem>(&known_id(*k), KF_FLAG_DEFAULT, None) }.ok(),
        Some(Start::Path(p)) => item_at(p),
        None => None,
    };
    if let Some(item) = start {
        // A start folder that is gone is not worth failing the dialog over.
        let _ = unsafe { if n.force_start { dialog.SetFolder(&item) } else { dialog.SetDefaultFolder(&item) } };
    }
    if let Some(name) = &n.suggested_name {
        let _ = unsafe { dialog.SetFileName(&HSTRING::from(name.as_str())) };
    }
    if let Some(ext) = &n.default_ext {
        let _ = unsafe { dialog.SetDefaultExtension(&HSTRING::from(ext.as_str())) };
    }

    let hwnd = (owner != 0).then(|| HWND(owner as *mut _));
    if let Err(e) = unsafe { dialog.Show(hwnd) } {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(None);
        }
        return Err(fail("Show", &e));
    }

    let mut out = Vec::new();
    match n.kind {
        Kind::Open | Kind::Folder => {
            let open: IFileOpenDialog = dialog.cast().map_err(|e| fail("IFileOpenDialog", &e))?;
            let items = unsafe { open.GetResults() }.map_err(|e| fail("GetResults", &e))?;
            let count = unsafe { items.GetCount() }.map_err(|e| fail("GetCount", &e))?;
            for i in 0..count {
                let item = unsafe { items.GetItemAt(i) }.map_err(|e| fail("GetItemAt", &e))?;
                out.push(path_of(&item)?);
            }
        }
        Kind::Save => {
            let item = unsafe { dialog.GetResult() }.map_err(|e| fail("GetResult", &e))?;
            out.push(path_of(&item)?);
        }
    }
    Ok(Some(out))
}

pub(super) fn color(value: &str, owner: isize, cancel: &Cancellation) -> Result<Option<String>, Fail> {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::Controls::Dialogs::{ChooseColorW, CommDlgExtendedError, CHOOSECOLORW, CC_FULLOPEN, CC_RGBINIT};
    if cancel.is_requested() { return Ok(None); }
    let _timer = CancellationTimer::new(cancel.clone())?;
    let number = u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(0);
    let rgb = ((number & 0xff) << 16) | (number & 0xff00) | ((number >> 16) & 0xff);
    let mut custom = [COLORREF(0xffffff); 16];
    let mut options = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        rgbResult: COLORREF(rgb),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_FULLOPEN | CC_RGBINIT,
        ..Default::default()
    };
    if unsafe { ChooseColorW(&mut options) }.as_bool() {
        let value = options.rgbResult.0;
        let rgb = ((value & 0xff) << 16) | (value & 0xff00) | ((value >> 16) & 0xff);
        Ok(Some(format!("#{rgb:06x}")))
    } else {
        let error = unsafe { CommDlgExtendedError() }.0;
        if error == 0 { Ok(None) } else {
            Err(Fail::new("InvalidStateError", format!("the color dialog failed ({error})")).code("EIO"))
        }
    }
}
