//! Default one-physical-device Windows host (`VN_GPU_SHARED=0` opts out). GPUI
//! retains its D3D11 pipelines through D3D11On12; redjs borrows the same wgpu
//! generation. Canvas display imports native GPU textures; VN_GPU_PRESENT=readback
//! selects the bounded readback bridge for controlled qualification.
use std::sync::{Arc, Mutex};

use anyhow::{ensure, Context as _, Result};
use gpui_windows::{install_directx_device_factory, ExternalDirectXDevice};
use vn_runtime::redjs::webapi::webgpu::backend::{
    self, Adapter, AdapterFuture, AdapterOptions, Context, GpuHost,
};
use windows::{
    core::{IUnknown, Interface},
    Win32::Graphics::{
        Direct3D::{D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1},
        Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, D3D11_CREATE_DEVICE_BGRA_SUPPORT},
        Direct3D11on12::{D3D11On12CreateDevice, ID3D11On12Device1},
        Direct3D12::ID3D12Device,
        Dxgi::{CreateDXGIFactory2, IDXGIDevice, IDXGIFactory6, DXGI_CREATE_FACTORY_FLAGS},
    },
};

mod presentation;

static CURRENT: Mutex<Option<Arc<Context>>> = Mutex::new(None);

struct SharedHost(Arc<Context>);
impl GpuHost for SharedHost {
    fn request_adapter(&self, options: AdapterOptions) -> AdapterFuture {
        let context = self.0.clone();
        Box::pin(async move {
            if options.force_fallback_adapter
                && context.adapter.get_info().device_type != wgpu::DeviceType::Cpu
            {
                return Ok(None);
            }
            // Power preference is a hint: renderer and JS must use one adapter.
            Ok(Some(Adapter {
                raw: context.adapter.clone(),
                shared: Some(context),
            }))
        })
    }
}

pub fn install() -> Result<()> {
    if shared_device_enabled(std::env::var("VN_GPU_SHARED").ok().as_deref()) {
        install_directx_device_factory(create_generation)?;
    }
    Ok(())
}

fn create_generation() -> Result<ExternalDirectXDevice> {
    match create_shared_generation() {
        Ok(device) => Ok(device),
        Err(error) => {
            // GPUI can fall back to its native D3D11 renderer. A failed shared
            // generation must not leave WebGPU bound to the retired device.
            vn_runtime::redjs::webapi::webgpu::canvas::set_presenter(None);
            backend::set_host(None);
            Err(error)
        }
    }
}

fn create_shared_generation() -> Result<ExternalDirectXDevice> {
    let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    // A factory call is a new GPUI generation, including after device recovery.
    // Retained JS objects must never silently migrate to the replacement device.
    if let Some(previous) = current.take() {
        previous.notify_lost(
            "unknown",
            "Host renderer replaced its shared GPU generation",
        );
    }
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(descriptor);
    let adapters = instance.enumerate_adapters(wgpu::Backends::DX12);
    let wanted = std::env::var("VN_ADAPTER")
        .unwrap_or_default()
        .to_lowercase();
    let mut candidates = pollster::block_on(adapters);
    candidates.sort_by_key(|a| {
        let info = a.get_info();
        let preference = match info.device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            _ => 3,
        };
        (
            !wanted.is_empty() && !info.name.to_lowercase().contains(&wanted),
            preference,
        )
    });
    let adapter = Arc::new(candidates.into_iter().next().context("No D3D12 adapter")?);
    if !wanted.is_empty() && !adapter.get_info().name.to_lowercase().contains(&wanted) {
        log::warn!(
            "VN_ADAPTER did not match a D3D12 adapter; using {}",
            adapter.get_info().name
        );
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vue-native shared renderer/WebGPU"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))?;
    let context = Arc::new(Context::from_shared(
        adapter,
        Arc::new(device),
        Arc::new(queue),
    ));
    let weak = Arc::downgrade(&context);
    context.device.set_device_lost_callback(move |_, message| {
        if let Some(context) = weak.upgrade() {
            context.notify_lost("unknown", message);
        }
    });
    let (raw_device, raw_queue) = unsafe {
        let hal = context
            .device
            .as_hal::<wgpu::hal::api::Dx12>()
            .context("Shared device is not D3D12")?;
        (hal.raw_device().clone(), hal.raw_queue().clone())
    };
    let (device, device_context) = unsafe {
        let mut device: Option<ID3D11Device> = None;
        let mut immediate: Option<ID3D11DeviceContext> = None;
        let mut level = D3D_FEATURE_LEVEL::default();
        D3D11On12CreateDevice(
            &raw_device,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT.0,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            Some(&[Some(raw_queue.cast::<IUnknown>()?)]),
            0,
            Some(&mut device),
            Some(&mut immediate),
            Some(&mut level),
        )?;
        (
            device.context("No D3D11On12 device")?,
            immediate.context("No D3D11On12 immediate context")?,
        )
    };
    // Validate GPUI's required interface before publishing the shared host.
    let multithread = device.cast::<ID3D11Multithread>().context("Shared renderer requires D3D11 multithread protection")?;
    let previously_protected = unsafe { multithread.SetMultithreadProtected(true) };
    log::debug!("Shared D3D11 multithread protection was {}", previously_protected.as_bool());
    let underlying: ID3D12Device = unsafe { device.cast::<ID3D11On12Device1>()?.GetD3D12Device()? };
    ensure!(
        underlying.cast::<IUnknown>()? == raw_device.cast::<IUnknown>()?,
        "Renderer and WebGPU device identity mismatch"
    );
    let adapter = unsafe { device.cast::<IDXGIDevice>()?.GetAdapter()?.cast()? };
    let dxgi_factory: IDXGIFactory6 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
    if std::env::var("VN_GPU_PRESENT").as_deref() != Ok("readback") {
        vn_runtime::redjs::webapi::webgpu::canvas::set_presenter(Some(Arc::new(
            presentation::Presenter::new(context.clone(), device.clone(), device_context.clone())?,
        )));
    } else {
        vn_runtime::redjs::webapi::webgpu::canvas::set_presenter(None);
    }
    backend::set_host(Some(Arc::new(SharedHost(context.clone()))));
    log::info!(
        "WebGPU/GPUI share the same physical device via D3D11On12: {}",
        context.adapter.get_info().name
    );
    *current = Some(context.clone());
    Ok(ExternalDirectXDevice {
        adapter,
        dxgi_factory,
        device,
        device_context,
        keep_alive: context,
    })
}

/// The old opt-in stays compatible; only an explicit diagnostic opt-out disables sharing.
fn shared_device_enabled(setting: Option<&str>) -> bool {
    setting != Some("0")
}

#[cfg(test)]
mod default_policy_tests {
    #[test]
    fn shared_gpu_is_default_and_only_zero_disables_it() {
        assert!(super::shared_device_enabled(None));
        assert!(super::shared_device_enabled(Some("1")));
        assert!(super::shared_device_enabled(Some("")));
        assert!(!super::shared_device_enabled(Some("0")));
    }
}
