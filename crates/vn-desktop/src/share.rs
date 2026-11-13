//! `navigator.share` through the Windows share sheet. Shared files are written to a per-process temp
//! folder, emptied at the next share.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;

use windows::core::{IInspectable, Interface, HSTRING};
use windows::ApplicationModel::DataTransfer::{DataPackage, DataRequestedEventArgs, DataTransferManager, ShareCompletedEventArgs};
use windows_collections::IIterable;
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::IDataTransferManagerInterop;

use crate::Fail;

pub struct SharedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Request {
    pub title: String,
    pub text: Option<String>,
    pub url: Option<String>,
    pub files: Vec<SharedFile>,
}

struct Pending {
    token: u32,
    title: String,
    text: Option<String>,
    url: Option<String>,
    files: Vec<StorageFile>,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static MANAGERS: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

fn folder() -> PathBuf {
    std::env::temp_dir().join(format!("vn-share-{}", std::process::id()))
}

/// A name that stays inside the folder.
fn safe_name(name: &str, i: usize) -> String {
    let clean: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let clean = clean.trim().trim_matches('.');
    if clean.is_empty() {
        format!("file-{i}")
    } else {
        clean.to_string()
    }
}

fn store(files: Vec<SharedFile>) -> windows::core::Result<Vec<StorageFile>> {
    let dir = folder();
    let _ = std::fs::remove_dir_all(&dir);
    let mut out = Vec::new();
    for (i, f) in files.into_iter().enumerate() {
        let sub = dir.join(i.to_string());
        let path = sub.join(safe_name(&f.name, i));
        std::fs::create_dir_all(&sub).and_then(|_| std::fs::write(&path, &f.bytes)).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x8000_4005u32 as i32), e.to_string()))?;
        out.push(StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.join()?);
    }
    Ok(out)
}

fn fill(args: &DataRequestedEventArgs) -> windows::core::Result<()> {
    let Some(p) = PENDING.with(|p| p.borrow_mut().take()) else { return Ok(()) };
    let token = p.token;
    let data = args.Request()?.Data()?;
    data.Properties()?.SetTitle(&HSTRING::from(&p.title))?;
    if let Some(t) = &p.text {
        data.SetText(&HSTRING::from(t))?;
    }
    if let Some(u) = &p.url {
        data.SetWebLink(&Uri::CreateUri(&HSTRING::from(u))?)?;
    }
    if !p.files.is_empty() {
        let items: Vec<Option<IStorageItem>> = p.files.iter().map(|f| f.cast().ok()).collect();
        data.SetStorageItems(&IIterable::<IStorageItem>::from(items), true)?;
    }
    data.ShareCompleted(&TypedEventHandler::<DataPackage, ShareCompletedEventArgs>::new(move |_, _| {
        crate::settle(token, Ok("{}".into()));
        Ok(())
    }))?;
    data.ShareCanceled(&TypedEventHandler::<DataPackage, IInspectable>::new(move |_, _| {
        crate::settle(token, Err(Fail::new("AbortError", "Share canceled")));
        Ok(())
    }))?;
    Ok(())
}

fn manager(hwnd: HWND) -> windows::core::Result<(IDataTransferManagerInterop, DataTransferManager)> {
    // SAFETY: COM on the window's thread; an initialised apartment is fine.
    let interop = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?
    };
    // SAFETY: `hwnd` is a live top-level window of this thread.
    let m: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
    if !MANAGERS.with(|ms| ms.borrow().contains(&(hwnd.0 as isize))) {
        m.DataRequested(&TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(|_, args| {
            if let Some(a) = args.as_ref() {
                if let Err(e) = fill(a) {
                    crate::advise(&format!("navigator.share: the share sheet refused the data ({e})"));
                }
            }
            Ok(())
        }))?;
        MANAGERS.with(|ms| ms.borrow_mut().push(hwnd.0 as isize));
    }
    Ok((interop, m))
}

/// The token settled when the sheet completes or is dismissed.
pub fn start(hwnd: isize, req: Request) -> Result<u32, Fail> {
    let token = crate::next_token();
    if let Some(prev) = PENDING.with(|p| p.borrow_mut().take()) {
        crate::settle(prev.token, Err(Fail::new("AbortError", "Share canceled")));
    }
    let hwnd = HWND(hwnd as *mut c_void);
    let shown = (|| -> windows::core::Result<()> {
        let files = store(req.files)?;
        let (interop, _m) = manager(hwnd)?;
        PENDING.with(|p| *p.borrow_mut() = Some(Pending { token, title: req.title, text: req.text, url: req.url, files }));
        // SAFETY: as in `manager`.
        unsafe { interop.ShowShareUIForWindow(hwnd) }
    })();
    match shown {
        Ok(()) => Ok(token),
        Err(e) => {
            PENDING.with(|p| p.borrow_mut().take());
            Err(Fail::new("NotAllowedError", format!("The share sheet could not be shown ({e})")))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_names_stay_in_the_folder() {
        assert_eq!(super::safe_name("photo.png", 0), "photo.png");
        assert_eq!(super::safe_name("../../evil.exe", 1), "evil.exe");
        assert_eq!(super::safe_name("a\\b:c", 2), "abc");
        assert_eq!(super::safe_name("..", 3), "file-3");
    }

    #[test]
    fn shared_files_are_storage_files() {
        let files = super::store(vec![super::SharedFile { name: "note.txt".into(), bytes: b"hi".to_vec() }]).unwrap();
        assert_eq!(files[0].Name().unwrap(), "note.txt");
        assert_eq!(std::fs::read(super::folder().join("0").join("note.txt")).unwrap(), b"hi");
        let _ = std::fs::remove_dir_all(super::folder());
    }
}
