//! The Windows shell calls, on the `sk-shell` STA thread. Open uses `ShellExecuteExW` with
//! `SEE_MASK_FLAG_NO_UI`, so "nothing is associated" returns `ERROR_NO_ASSOCIATION` instead of a dialog.
//! Trash uses `IFileOperation` with a sink whose `PreDeleteItem` vetoes any delete not routed through the
//! Recycle Bin, so "trash" never quietly becomes a permanent delete.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use windows::core::{implement, Ref, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, E_ABORT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::{
    FileOperation, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
    ILCreateFromPathW, ILFree, IShellItem, SHCreateItemFromParsingName, SHOpenFolderAndSelectItems,
    ShellExecuteExW, FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO,
    FOF_NO_UI, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    TSF_DELETE_RECYCLE_IF_POSSIBLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNORMAL,
};

use crate::Fail;

/// A COM single-threaded apartment, which `IFileOperation` and the folder views want.
pub fn thread_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
}

fn drain_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\s\x` → `\\s\x`: `IShellItem` parsing does not understand the long-path prefix.
fn shell_path(p: &Path) -> HSTRING {
    let s = p.as_os_str().to_string_lossy();
    let s = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    };
    HSTRING::from(s)
}

fn win32_of(hr: HRESULT) -> Option<u32> {
    let v = hr.0 as u32;
    if v & 0xFFFF_0000 == 0x8007_0000 {
        Some(v & 0xFFFF)
    } else if v != 0 && v < 0x1_0000 {
        Some(v)
    } else {
        None
    }
}

fn fail(verb: &str, target: &str, err: &windows::core::Error) -> Fail {
    let hr = err.code();
    let os = win32_of(hr);
    let f = match os {
        Some(1155) | Some(1156) => Fail::new(
            "NotFoundError",
            format!("{verb}: no application is associated with {target}"),
        )
        .code("ENOASSOC"),
        Some(2) | Some(3) => Fail::new(
            "NotFoundError",
            format!(
                "{verb}: {target} — a requested file or directory could not be found at the time an operation was processed."
            ),
        )
        .code("ENOENT"),
        Some(5) => Fail::new("NotAllowedError", format!("{verb}: {target} — access is denied")).code("EACCES"),
        Some(32) | Some(33) => Fail::new(
            "NoModificationAllowedError",
            format!("{verb}: {target} is in use by another program"),
        )
        .code("EBUSY"),
        Some(1223) => Fail::new("AbortError", format!("{verb}: {target} — the operation was cancelled")).code("ECANCELED"),
        _ => Fail::new("InvalidStateError", format!("{verb}: {target} — {}", err.message())).code("EIO"),
    };
    match os {
        Some(code) => f.os(code as i32),
        None => f.os(hr.0),
    }
}

fn shell_execute(file: &HSTRING, directory: Option<&HSTRING>) -> windows::core::Result<()> {
    let verb = HSTRING::from("open");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: directory.map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let r = unsafe { ShellExecuteExW(&mut info) };
    drain_messages();
    r
}

pub fn open_external(url: &str) -> Result<(), Fail> {
    shell_execute(&HSTRING::from(url), None).map_err(|e| fail("openExternal", url, &e))
}

pub fn open_path(path: &Path) -> Result<(), Fail> {
    let dir = path.parent().map(shell_path);
    shell_execute(&shell_path(path), dir.as_ref())
        .map_err(|e| fail("openPath", &path.display().to_string(), &e))
}

struct Pidl(*mut windows::Win32::UI::Shell::Common::ITEMIDLIST);

impl Pidl {
    fn of(p: &Path) -> Option<Pidl> {
        let s = shell_path(p);
        let raw = unsafe { ILCreateFromPathW(PCWSTR(s.as_ptr())) };
        (!raw.is_null()).then_some(Pidl(raw))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { ILFree(Some(self.0)) };
    }
}

pub fn show_item(path: &Path) -> Result<(), Fail> {
    let target = path.display().to_string();
    let not_found = || {
        Fail::new(
            "NotFoundError",
            format!(
                "showItemInFolder: {target} — a requested file or directory could not be found at the time an operation was processed."
            ),
        )
        .code("ENOENT")
    };
    let Some(parent) = path.parent() else {
        return shell_execute(&shell_path(path), None)
            .map_err(|e| fail("showItemInFolder", &target, &e));
    };
    let dir = Pidl::of(parent).ok_or_else(not_found)?;
    let item = Pidl::of(path).ok_or_else(not_found)?;
    let selection = [item.0 as *const _];
    let r = unsafe { SHOpenFolderAndSelectItems(dir.0, Some(&selection), 0) };
    drain_messages();
    match r {
        Ok(()) => Ok(()),
        // On some systems this fails with "file not found" although the file exists; opening the folder still works.
        Err(e) if win32_of(e.code()) == Some(ERROR_FILE_NOT_FOUND.0) => {
            shell_execute(&shell_path(parent), None)
                .map_err(|e| fail("showItemInFolder", &target, &e))
        }
        Err(e) => Err(fail("showItemInFolder", &target, &e)),
    }
}

#[implement(IFileOperationProgressSink)]
struct TrashSink {
    vetoed: Arc<AtomicBool>,
    item_error: Arc<AtomicI32>,
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for TrashSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hr: HRESULT) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        _n: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        _n: &PCWSTR,
        _hr: HRESULT,
        _c: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
        _hr: HRESULT,
        _c: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
        _hr: HRESULT,
        _c: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreDeleteItem(&self, dwflags: u32, _i: Ref<IShellItem>) -> windows::core::Result<()> {
        if dwflags & (TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32) == 0 {
            self.vetoed.store(true, Ordering::SeqCst);
            return Err(E_ABORT.into());
        }
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _f: u32,
        _i: Ref<IShellItem>,
        hr: HRESULT,
        _c: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        if hr.is_err() {
            let _ = self
                .item_error
                .compare_exchange(0, hr.0, Ordering::SeqCst, Ordering::SeqCst);
        }
        Ok(())
    }
    fn PreNewItem(&self, _f: u32, _d: Ref<IShellItem>, _n: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _f: u32,
        _d: Ref<IShellItem>,
        _n: &PCWSTR,
        _t: &PCWSTR,
        _a: u32,
        _hr: HRESULT,
        _c: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _total: u32, _so_far: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

pub fn trash(path: &Path) -> Result<(), Fail> {
    let target = path.display().to_string();
    let vetoed = Arc::new(AtomicBool::new(false));
    let item_error = Arc::new(AtomicI32::new(0));
    let result = (|| -> windows::core::Result<bool> {
        unsafe {
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(
                FOF_NO_UI
                    | FOF_ALLOWUNDO
                    | FOFX_ADDUNDORECORD
                    | FOFX_RECYCLEONDELETE
                    | FOFX_EARLYFAILURE,
            )?;
            let item: IShellItem = SHCreateItemFromParsingName(&shell_path(path), None)?;
            let sink: IFileOperationProgressSink = TrashSink {
                vetoed: vetoed.clone(),
                item_error: item_error.clone(),
            }
            .into();
            op.DeleteItem(&item, &sink)?;
            let performed = op.PerformOperations();
            let aborted = op
                .GetAnyOperationsAborted()
                .map(|b| b.as_bool())
                .unwrap_or(false);
            performed?;
            Ok(aborted)
        }
    })();
    drain_messages();
    if vetoed.load(Ordering::SeqCst) {
        return Err(Fail::new(
            "InvalidStateError",
            format!(
                "trashItem: {target} cannot go to the Recycle Bin (the drive has none, or the item is too large for it) — nothing was deleted"
            ),
        )
        .code("ENOTRASH"));
    }
    let item_hr = item_error.load(Ordering::SeqCst);
    if item_hr != 0 {
        return Err(fail(
            "trashItem",
            &target,
            &windows::core::Error::from(HRESULT(item_hr)),
        ));
    }
    match result {
        Ok(false) => Ok(()),
        Ok(true) => Err(Fail::new(
            "AbortError",
            format!("trashItem: {target} — the operation was aborted"),
        )
        .code("ECANCELED")),
        Err(e) => Err(fail("trashItem", &target, &e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_path_prefixes_are_stripped_for_the_shell() {
        assert_eq!(
            shell_path(Path::new(r"\\?\C:\a\b.txt")).to_string(),
            r"C:\a\b.txt"
        );
        assert_eq!(
            shell_path(Path::new(r"\\?\UNC\srv\share\b.txt")).to_string(),
            r"\\srv\share\b.txt"
        );
        assert_eq!(shell_path(Path::new(r"C:\a")).to_string(), r"C:\a");
    }

    #[test]
    fn win32_codes_come_out_of_either_encoding() {
        assert_eq!(win32_of(HRESULT(0x8007_0483_u32 as i32)), Some(1155));
        assert_eq!(win32_of(HRESULT(2)), Some(2));
        assert_eq!(win32_of(E_ABORT), None);
        let f = fail(
            "openPath",
            "x.xyz",
            &windows::core::Error::from(HRESULT(0x8007_0483_u32 as i32)),
        );
        assert_eq!((f.name, f.code), ("NotFoundError", Some("ENOASSOC")));
    }

    /// Not in the default run: it leaves a Recycle Bin entry the test must purge. `-- --ignored trash_moves`.
    #[test]
    #[ignore]
    fn trash_moves_a_file_to_the_recycle_bin() {
        thread_init();
        let dir = std::env::temp_dir().join(format!("sk-desktop-trash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sk-desktop-trash-probe.txt");
        let marker = format!("sk-desktop trash probe {}", std::process::id());
        std::fs::write(&file, &marker).unwrap();
        trash(&file).expect("trash");
        assert!(!file.exists(), "the file is gone from its folder");
        let _ = std::fs::remove_dir(&dir);
        let drive = file.to_string_lossy()[..2].to_string();
        let bin = Path::new(&format!(r"{drive}\")).join("$Recycle.Bin");
        let mut purged = false;
        if let Ok(sids) = std::fs::read_dir(&bin) {
            for sid in sids.flatten() {
                let Ok(items) = std::fs::read_dir(sid.path()) else {
                    continue;
                };
                for it in items.flatten() {
                    let name = it.file_name().to_string_lossy().into_owned();
                    if name.starts_with("$R")
                        && std::fs::read_to_string(it.path()).ok().as_deref()
                            == Some(marker.as_str())
                    {
                        let _ = std::fs::remove_file(it.path());
                        let _ = std::fs::remove_file(sid.path().join(name.replacen("$R", "$I", 1)));
                        purged = true;
                    }
                }
            }
        }
        assert!(
            purged,
            "the probe was found in (and purged from) the Recycle Bin"
        );
    }
}
