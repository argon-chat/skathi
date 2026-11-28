//! The Windows share sheet, through the window's `DataTransferManager`.

use std::cell::RefCell;
use std::ffi::c_void;

use sk_windows::core::{Interface, HSTRING};
use sk_windows::runtime::{
    DataRequestedEventArgs, DataTransferManager, IStorageItem, StorageFile, Uri,
};
use sk_windows::win32::{
    CoInitializeEx, IDataTransferManagerInterop, COINIT_APARTMENTTHREADED, HWND,
};
use windows_collections::IIterable;

use super::{write, Request, SharedFile};
use crate::Fail;

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

fn store(files: Vec<SharedFile>) -> sk_windows::core::Result<Vec<StorageFile>> {
    let paths = write(files).map_err(|e| {
        sk_windows::core::Error::new(
            sk_windows::core::HRESULT(0x8000_4005u32 as i32),
            e.to_string(),
        )
    })?;
    paths
        .iter()
        .map(|p| StorageFile::GetFileFromPathAsync(&HSTRING::from(p.as_os_str()))?.join())
        .collect()
}

fn fill(args: &DataRequestedEventArgs) -> sk_windows::core::Result<()> {
    let Some(p) = PENDING.with(|p| p.borrow_mut().take()) else {
        return Ok(());
    };
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
        data.SetStorageItems(&storage_items(&p.files)?, true)?;
    }
    // The OS owns the data package until the share sheet completes.
    let _ = data
        .ShareCompleted(move |_, _| crate::settle(token, Ok("{}".into())))?
        .into_token();
    let _ = data
        .ShareCanceled(move |_, _| {
            crate::settle(token, Err(Fail::new("AbortError", "Share canceled")));
        })?
        .into_token();
    Ok(())
}

fn storage_items(files: &[StorageFile]) -> sk_windows::core::Result<IIterable<IStorageItem>> {
    let items = files
        .iter()
        .map(|file| file.cast::<IStorageItem>().map(Some));
    Ok(IIterable::from(
        items.collect::<sk_windows::core::Result<Vec<_>>>()?,
    ))
}

fn manager(
    hwnd: HWND,
) -> sk_windows::core::Result<(IDataTransferManagerInterop, DataTransferManager)> {
    // SAFETY: COM on the window's thread; an initialised apartment is fine.
    let interop = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED as u32);
        sk_windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?
    };
    // SAFETY: `hwnd` is a live top-level window of this thread.
    let m: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
    if !MANAGERS.with(|ms| ms.borrow().contains(&(hwnd.0 as isize))) {
        let _ = m
            .DataRequested(|_, args| {
                if let Some(a) = args.as_ref() {
                    if let Err(e) = fill(a) {
                        crate::advise(&format!(
                            "navigator.share: the share sheet refused the data ({e})"
                        ));
                    }
                }
            })?
            .into_token();
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
    let shown = (|| -> sk_windows::core::Result<()> {
        let files = store(req.files)?;
        let (interop, _m) = manager(hwnd)?;
        PENDING.with(|p| {
            *p.borrow_mut() = Some(Pending {
                token,
                title: req.title,
                text: req.text,
                url: req.url,
                files,
            })
        });
        // SAFETY: as in `manager`.
        unsafe { interop.ShowShareUIForWindow(hwnd) }.ok()
    })();
    match shown {
        Ok(()) => Ok(token),
        Err(e) => {
            PENDING.with(|p| p.borrow_mut().take());
            Err(Fail::new(
                "NotAllowedError",
                format!("The share sheet could not be shown ({e})"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_files_are_storage_files() {
        let folder = crate::share::folder();
        let files = super::store(vec![super::SharedFile {
            name: "note.txt".into(),
            bytes: b"hi".to_vec(),
        }])
        .unwrap();
        assert_eq!(files[0].Name().unwrap(), "note.txt");
        let items = super::storage_items(&files).unwrap();
        sk_windows::runtime::DataPackage::new()
            .unwrap()
            .SetStorageItems(&items, true)
            .unwrap();
        assert_eq!(
            std::fs::read(folder.join("0").join("note.txt")).unwrap(),
            b"hi"
        );
        let _ = std::fs::remove_dir_all(folder);
    }
}
