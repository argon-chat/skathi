//! The renderer's D3D11 device as `<video>` and `<sk-surface>` borrow it. Lives here rather than in
//! sk-video so `<sk-surface>` keeps it in a build without the `video` feature.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;

static DEVICE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static GENERATION: AtomicU64 = AtomicU64::new(0);
static PROVIDER: OnceLock<SharedDeviceProvider> = OnceLock::new();

/// An owned lease on a renderer generation, held only while cloning the D3D11 interface so a
/// concurrent device replacement cannot invalidate the raw pointer before AddRef.
pub struct SharedVideoDevice {
    pub device: *mut c_void,
    pub generation: u64,
    pub keep_alive: Arc<dyn std::any::Any + Send + Sync>,
}

pub type SharedDeviceProvider = fn() -> Result<Option<SharedVideoDevice>, String>;

/// # Safety
/// A successful provider lease must own a live ID3D11Device through keep_alive.
pub unsafe fn install_provider(provider: SharedDeviceProvider) -> Result<(), &'static str> {
    PROVIDER
        .set(provider)
        .map_err(|_| "video device provider already installed")?;
    forward_to_video();
    Ok(())
}

/// # Safety
/// `ptr` is null or a live `ID3D11Device` COM pointer owned by the caller for the renderer's life.
pub unsafe fn set_device(ptr: *mut c_void, generation: u64) {
    DEVICE.store(ptr, Ordering::Release);
    GENERATION.store(generation, Ordering::Release);
    forward_to_video();
}

/// The renderer's D3D11 device and its generation, `None` before a renderer exists.
pub fn renderer_device() -> Result<Option<SharedVideoDevice>, String> {
    if let Some(provider) = PROVIDER.get() {
        return provider();
    }
    let raw = DEVICE.load(Ordering::Acquire);
    if raw.is_null() {
        return Ok(None);
    }
    // SAFETY: the host publishes a device it owns for the renderer's life.
    let Some(device) = (unsafe { ID3D11Device::from_raw_borrowed(&raw) }) else {
        return Ok(None);
    };
    let device = device.clone();
    Ok(Some(SharedVideoDevice {
        device: device.as_raw(),
        generation: GENERATION.load(Ordering::Acquire),
        keep_alive: Arc::new(SendDevice(device)),
    }))
}

struct SendDevice(#[allow(dead_code)] ID3D11Device);
// SAFETY: the renderer's device is multithread-protected; this only holds a reference.
unsafe impl Send for SendDevice {}
unsafe impl Sync for SendDevice {}

#[cfg(feature = "video")]
fn forward_to_video() {
    fn lease() -> Result<Option<sk_video::mf::SharedVideoDevice>, String> {
        renderer_device().map(|lease| {
            lease.map(|l| sk_video::mf::SharedVideoDevice {
                device: l.device,
                generation: l.generation,
                keep_alive: l.keep_alive,
            })
        })
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: `lease` returns this module's leases, which own their device.
    ONCE.call_once(|| unsafe {
        let _ = sk_video::mf::install_shared_device_provider(lease);
    });
}

#[cfg(not(feature = "video"))]
fn forward_to_video() {}
