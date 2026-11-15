//! Graphics interoperability selected without allocating a WebGPU device.
//! Native D3D12 owns the renderer; WebGPU joins its generation on first use.

#[cfg(not(feature = "native-dx12"))]
#[path = "shared_gpu/legacy.rs"]
mod implementation;
#[cfg(feature = "native-dx12")]
#[path = "shared_gpu/native.rs"]
mod implementation;

pub use implementation::install;

pub(crate) struct Stats {
    pub generation: u64,
    pub contexts: u64,
    pub converters: u64,
    pub frames: u64,
    pub video_bridge_active: bool,
}

pub(crate) fn stats() -> Option<Stats> {
    #[cfg(feature = "native-dx12")]
    {
        Some(implementation::stats())
    }
    #[cfg(not(feature = "native-dx12"))]
    {
        None
    }
}

fn shared_device_enabled(setting: Option<&str>) -> bool {
    setting != Some("0")
}
