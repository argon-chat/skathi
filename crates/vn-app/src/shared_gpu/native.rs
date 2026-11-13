//! A weak, lazy WebGPU guest of the already-created native renderer device.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock, Weak,
};

use anyhow::{ensure, Context as _, Result};
use gpui_windows::{install_renderer_d3d12_generation_observer, renderer_d3d12_device};
use vn_runtime::redjs::webapi::webgpu::{
    backend::{self, Adapter, AdapterFuture, AdapterOptions, Context, GpuHost},
    canvas,
};
use windows::{
    core::{IUnknown, Interface},
    Win32::Graphics::Direct3D12::ID3D12Device,
};

#[path = "native_presentation.rs"]
mod presentation;

static STATE: OnceLock<Arc<HostState>> = OnceLock::new();

static CONTEXTS: AtomicU64 = AtomicU64::new(0);
static CONVERTERS: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);

struct Lifetime(&'static AtomicU64);
impl Lifetime {
    fn new(counter: &'static AtomicU64) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

struct RendererOwner {
    _physical: Arc<dyn std::any::Any + Send + Sync>,
    _lifetime: Lifetime,
}

pub(super) fn stats() -> super::Stats {
    let generation = STATE.get().map_or(0, |state| {
        state
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .number
    });
    super::Stats {
        generation,
        contexts: CONTEXTS.load(Ordering::Relaxed),
        converters: CONVERTERS.load(Ordering::Relaxed),
        frames: FRAMES.load(Ordering::Relaxed),
        video_bridge_active: !gpui_windows::renderer_d3d11_device().0.is_null(),
    }
}

fn ensure_generation(number: u64, context: &Context) -> Result<()> {
    if !renderer_d3d12_device().is_some_and(|renderer| renderer.generation == number) {
        context.notify_lost(
            "unknown",
            "Renderer generation changed during WebGPU request",
        );
        anyhow::bail!("Renderer generation changed during WebGPU request");
    }
    Ok(())
}

#[derive(Default)]
struct Generation {
    number: u64,
    context: Weak<Context>,
}

#[derive(Default)]
struct HostState {
    generation: Mutex<Generation>,
}

impl HostState {
    fn retire(&self, number: u64) {
        let mut current = self
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if current.number != number {
            if let Some(previous) = current.context.upgrade() {
                previous.notify_lost(
                    "unknown",
                    "Host renderer replaced its native D3D12 generation",
                );
            }
            current.number = number;
            current.context = Weak::new();
        }
    }

    fn current_generation(&self, context: &Arc<Context>) -> Option<u64> {
        let renderer = renderer_d3d12_device()?;
        self.retire(renderer.generation);
        let current = self
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let shared = current.context.upgrade()?;
        Arc::ptr_eq(&shared, context).then_some(current.number)
    }

    fn request_adapter(&self, options: AdapterOptions) -> Result<Option<Adapter>> {
        let renderer =
            renderer_d3d12_device().context("The native renderer has no active D3D12 device")?;
        self.retire(renderer.generation);
        // Serialize only first-use guest creation. This is intentionally not a
        // host startup action; an ordinary chat never creates a wgpu instance.
        let mut current = self
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(context) = current.context.upgrade() {
            if options.force_fallback_adapter
                && context.adapter.get_info().device_type != wgpu::DeviceType::Cpu
            {
                return Ok(None);
            }
            if let Some(loss) = context.loss() {
                anyhow::bail!("Shared GPU generation is lost: {}", loss.message);
            }
            ensure_generation(current.number, &context)?;
            return Ok(Some(Adapter {
                raw: context.adapter.clone(),
                shared: Some(context),
            }));
        }
        let physical = unsafe { ID3D12Device::from_raw_borrowed(&renderer.device) }
            .context("Renderer returned an invalid D3D12 device")?
            .clone();
        let wanted_luid = unsafe { physical.GetAdapterLuid() };
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        // Debug layers must be configured before the platform creates its
        // device. Enabling them again while joining an existing device can
        // remove it; wgpu's API validation remains enabled independently.
        descriptor
            .flags
            .remove(wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::GPU_BASED_VALIDATION);
        let instance = wgpu::Instance::new(descriptor);
        let candidates = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
        let mut matching = None;
        for adapter in candidates {
            let matches = {
                let native = unsafe { adapter.as_hal::<wgpu::hal::api::Dx12>() }
                    .context("The selected WebGPU adapter is not D3D12")?;
                let description = unsafe { native.raw_adapter().GetDesc2()? };
                description.AdapterLuid.LowPart == wanted_luid.LowPart
                    && description.AdapterLuid.HighPart == wanted_luid.HighPart
            };
            if matches
                && (!options.force_fallback_adapter
                    || adapter.get_info().device_type == wgpu::DeviceType::Cpu)
            {
                matching = Some(adapter);
                break;
            }
        }
        let Some(adapter) = matching else {
            return Ok(None);
        };
        let adapter = Arc::new(adapter);
        // The renderer's adapter selection is authoritative. WebGPU's power
        // preference is a hint and cannot move a canvas to another device.
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("vue-native lazy shared D3D12 WebGPU guest"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            }))?;
        {
            let native = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }
                .context("The shared WebGPU device is not D3D12")?;
            ensure!(
                native.raw_device().cast::<IUnknown>()? == physical.cast::<IUnknown>()?,
                "WebGPU and the native renderer have different D3D12 device identities",
            );
        }
        let context = Arc::new(Context::from_shared_with_owner(
            adapter,
            Arc::new(device),
            Arc::new(queue),
            Arc::new(RendererOwner {
                _physical: renderer.keep_alive,
                _lifetime: Lifetime::new(&CONTEXTS),
            }),
        ));
        let weak = Arc::downgrade(&context);
        context.device.set_device_lost_callback(move |_, message| {
            if let Some(context) = weak.upgrade() {
                context.notify_lost("unknown", message);
            }
        });
        // Creation runs on redjs's request worker. Do not publish a device
        // whose renderer was replaced while the asynchronous request ran.
        ensure_generation(renderer.generation, &context)?;
        current.context = Arc::downgrade(&context);
        log::info!(
            "WebGPU lazily joined native D3D12 renderer generation {}: {}",
            current.number,
            context.adapter.get_info().name
        );
        Ok(Some(Adapter {
            raw: context.adapter.clone(),
            shared: Some(context),
        }))
    }
}

struct SharedHost(Arc<HostState>);
impl GpuHost for SharedHost {
    fn request_adapter(&self, options: AdapterOptions) -> AdapterFuture {
        let state = self.0.clone();
        Box::pin(async move {
            state
                .request_adapter(options)
                .map_err(|error| format!("{error:#}"))
        })
    }
}

fn generation_changed(number: u64) {
    if let Some(state) = STATE.get() {
        state.retire(number);
    }
}

pub fn install() -> Result<()> {
    if !super::shared_device_enabled(std::env::var("VN_GPU_SHARED").ok().as_deref()) {
        // Explicit diagnostic mode leaves redjs's independent lazy backend and
        // bounded readback presenter active. Native GPUI still owns D3D12.
        backend::set_host(None);
        canvas::set_presenter(None);
        return Ok(());
    }
    let state = STATE.get_or_init(|| Arc::new(HostState::default())).clone();
    install_renderer_d3d12_generation_observer(generation_changed)?;
    backend::set_host(Some(Arc::new(SharedHost(state.clone()))));
    if std::env::var("VN_GPU_PRESENT").as_deref() == Ok("readback") {
        canvas::set_presenter(None);
    } else {
        canvas::set_presenter(Some(Arc::new(presentation::Presenter::new(state))));
    }
    Ok(())
}
