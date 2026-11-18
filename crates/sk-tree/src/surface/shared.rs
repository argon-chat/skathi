//! A [`super::SharedTexture`]'s platform half. Under the D3D12 renderer it is opened on D3D12 and
//! copied there; a keyed mutex (a D3D11 object) and a legacy handle go through the D3D11 device.

use std::ffi::c_void;
use std::sync::{Arc, Mutex, Weak};

use windows::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE};

use super::d3d11::{self, SharedSync};
use super::{d3d12, lock, State, SurfaceError, SurfaceFormat};

struct Owned(HANDLE);

// SAFETY: a kernel handle is usable from any thread.
unsafe impl Send for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn raw(handle: u64) -> HANDLE {
    HANDLE(handle as usize as *mut c_void)
}

fn duplicate(process: HANDLE, handle: HANDLE) -> windows::core::Result<Owned> {
    let mut ours = HANDLE::default();
    unsafe {
        DuplicateHandle(
            process,
            handle,
            GetCurrentProcess(),
            &mut ours,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }?;
    Ok(Owned(ours))
}

fn from_process(pid: u32, handle: u64) -> Result<Owned, SurfaceError> {
    let process = unsafe { OpenProcess(PROCESS_DUP_HANDLE, false, pid) }.map_err(|e| {
        SurfaceError::Platform(format!("opening process {pid} to duplicate from: {e}"))
    })?;
    let ours = duplicate(process, raw(handle));
    unsafe {
        let _ = CloseHandle(process);
    }
    ours.map_err(|e| {
        SurfaceError::Platform(format!("duplicating handle {handle} of process {pid}: {e}"))
    })
}

pub(crate) struct Opened {
    on: Arc<Mutex<OnDevice>>,
    /// Ours, for the D3D11 open a keyed mutex needs after a D3D12 one.
    handle: Option<Owned>,
    pub(super) size: (u32, u32),
    pub(super) format: SurfaceFormat,
    pub(super) fenced: bool,
}

/// The texture as opened on the renderer's device, which the embedder holds: let go of when that
/// device is removed, so a new one can be made.
struct OnDevice {
    d3d12: Option<d3d12::Opened>,
    d3d11: Option<Result<d3d11::Opened, SurfaceError>>,
    released: bool,
}

static OPENED: Mutex<Vec<Weak<Mutex<OnDevice>>>> = Mutex::new(Vec::new());

/// The renderer's device was removed: every shared texture lets go of it.
pub(super) fn release_all() {
    let opened = std::mem::take(&mut *lock(&OPENED));
    for on in opened.iter().filter_map(Weak::upgrade) {
        let mut on = lock(&on);
        on.d3d12 = None;
        on.d3d11 = None;
        on.released = true;
    }
}

fn opened(
    d3d12: Option<d3d12::Opened>,
    d3d11: Option<Result<d3d11::Opened, SurfaceError>>,
) -> Arc<Mutex<OnDevice>> {
    let on = Arc::new(Mutex::new(OnDevice {
        d3d12,
        d3d11,
        released: false,
    }));
    let mut all = lock(&OPENED);
    all.retain(|w| w.strong_count() > 0);
    all.push(Arc::downgrade(&on));
    drop(all);
    // Removed while it was opened, after `release_all` ran.
    if super::removed() {
        release_all();
    }
    on
}

pub(super) fn open(handle: u64) -> Result<Opened, SurfaceError> {
    let handle = raw(handle);
    if d3d12::native() {
        if let Ok(opened_12) = d3d12::open(handle) {
            return Ok(Opened {
                size: opened_12.size,
                format: opened_12.format,
                on: opened(Some(opened_12), None),
                handle: duplicate(unsafe { GetCurrentProcess() }, handle).ok(),
                fenced: false,
            });
        }
    }
    let opened_11 = d3d11::open(handle)?;
    Ok(Opened {
        size: opened_11.size(),
        format: opened_11.format(),
        on: opened(None, Some(Ok(opened_11))),
        handle: None,
        fenced: false,
    })
}

pub(super) fn open_from_process(pid: u32, handle: u64) -> Result<Opened, SurfaceError> {
    let ours = from_process(pid, handle)?;
    open(ours.0 .0 as usize as u64)
}

impl Opened {
    pub(super) fn open_fence(&mut self, handle: u64) -> Result<(), SurfaceError> {
        let mut on = lock(&self.on);
        let on = &mut *on;
        match (&mut on.d3d12, &mut on.d3d11) {
            (Some(opened), _) => opened.open_fence(raw(handle))?,
            (None, Some(Ok(opened))) => opened.open_fence(raw(handle))?,
            (None, Some(Err(e))) => return Err(e.clone()),
            (None, None) => return Err(SurfaceError::DeviceChanged),
        }
        self.fenced = true;
        Ok(())
    }

    pub(super) fn open_fence_from_process(
        &mut self,
        pid: u32,
        handle: u64,
    ) -> Result<(), SurfaceError> {
        let ours = from_process(pid, handle)?;
        self.open_fence(ours.0 .0 as usize as u64)
    }

    /// A keyed mutex is a D3D11 object: open the texture there too.
    pub(super) fn need_d3d11(&mut self) {
        let mut on = lock(&self.on);
        if on.d3d11.is_none() && !on.released {
            on.d3d11 = Some(match &self.handle {
                Some(handle) => d3d11::open(handle.0),
                None => Err(SurfaceError::Platform(
                    "the shared handle could not be kept to open the keyed mutex".into(),
                )),
            });
        }
    }

    pub(super) fn present(
        &self,
        state: &Mutex<State>,
        sync: SharedSync,
        timeout_ms: u32,
    ) -> Result<u64, SurfaceError> {
        let on = lock(&self.on);
        if on.released {
            return Err(SurfaceError::DeviceChanged);
        }
        if let (Some(opened), false) = (&on.d3d12, matches!(sync, SharedSync::Keyed(..))) {
            let values = match sync {
                SharedSync::Fence(wait, release) => Some((wait, release)),
                _ => None,
            };
            return d3d12::present_shared(state, opened, values);
        }
        match &on.d3d11 {
            Some(Ok(opened)) => d3d11::present_shared(state, opened, sync, timeout_ms),
            Some(Err(e)) => Err(e.clone()),
            None => Err(SurfaceError::Platform(
                "the shared texture is not open".into(),
            )),
        }
    }
}
