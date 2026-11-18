//! The Metal half of a surface on macOS: frames are copied on a command queue of the surface's own
//! on the renderer's device, after a GPU wait on the producer's shared event, into one of a few
//! surface textures. The element's tick hands a finished one to the atlas
//! (`write_image_tile_metal`), so the renderer's queue never waits on a producer. A half-float
//! texture is first mapped into an 8-bit one of its own, on the same queue, when the tick asks.

use std::any::Any;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::Message;
use objc2_core_foundation::{CFRetained, CFString};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVPixelFormatType_64RGBAHalf,
    kCVReturnSuccess, CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetIOSurface, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::NSString;
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLComputeCommandEncoder, MTLComputePipelineState, MTLDevice, MTLLibrary, MTLOrigin,
    MTLPixelFormat, MTLResource, MTLSharedEvent, MTLSharedEventHandle, MTLSharedEventListener,
    MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};

use super::{check_format, check_size, lock, Frame, State, SurfaceError, SurfaceFormat};

/// More in flight than this means the atlas is not keeping up; the present is refused.
const MAX_SLOTS: usize = 4;

const BGRA: u32 = kCVPixelFormatType_32BGRA;
const NV12_VIDEO: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange;
const NV12_FULL: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;
const RGHA: u32 = kCVPixelFormatType_64RGBAHalf;

type Texture = ProtocolObject<dyn MTLTexture>;
type SharedEvent = ProtocolObject<dyn MTLSharedEvent>;

struct Gpu {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    renderer_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    generation: u64,
    nv12: OnceLock<Result<Pipeline, String>>,
    rgba16f: OnceLock<Result<Pipeline, String>>,
    _lease: Arc<dyn Any + Send + Sync>,
}

// SAFETY: Metal devices, queues and pipeline states are thread-safe.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

/// A surface's own command queue: a producer event that never fires stalls only this surface.
struct Queue {
    gpu: Arc<Gpu>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    /// Signalled after each copy: the release event the producer waits on.
    event: Retained<SharedEvent>,
    /// The last value signalled on `event`; held across each commit so values rise in queue order.
    next: Mutex<u64>,
}

// SAFETY: Metal queues and events are thread-safe; command buffers are encoded under `next`'s
// mutex.
unsafe impl Send for Queue {}
unsafe impl Sync for Queue {}

impl Queue {
    fn new(gpu: &Arc<Gpu>) -> Result<Arc<Self>, SurfaceError> {
        let queue = gpu
            .device
            .newCommandQueue()
            .ok_or_else(|| platform("creating the surface queue"))?;
        let event = gpu
            .device
            .newSharedEvent()
            .ok_or_else(|| platform("creating the release event"))?;
        Ok(Arc::new(Self {
            gpu: gpu.clone(),
            queue,
            event,
            next: Mutex::new(0),
        }))
    }
}

static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);

fn platform(what: &str) -> SurfaceError {
    SurfaceError::Platform(format!("{what} failed"))
}

fn gpu() -> Result<Arc<Gpu>, SurfaceError> {
    let lease = gpui::renderer_metal_device()
        .ok_or_else(|| SurfaceError::NoDevice("no window has drawn yet".into()))?;
    let mut cached = lock(&GPU);
    if let Some(gpu) = cached.as_ref().filter(|g| g.generation == lease.generation) {
        return Ok(gpu.clone());
    }
    // SAFETY: the lease keeps both alive.
    let device = unsafe { Retained::retain(lease.device.cast::<ProtocolObject<dyn MTLDevice>>()) }
        .ok_or_else(|| SurfaceError::NoDevice("the renderer returned a null device".into()))?;
    // SAFETY: as above.
    let renderer_queue =
        unsafe { Retained::retain(lease.queue.cast::<ProtocolObject<dyn MTLCommandQueue>>()) }
            .ok_or_else(|| SurfaceError::NoDevice("the renderer returned a null queue".into()))?;
    let gpu = Arc::new(Gpu {
        device,
        renderer_queue,
        generation: lease.generation,
        nv12: OnceLock::new(),
        rgba16f: OnceLock::new(),
        _lease: lease.keep_alive,
    });
    *cached = Some(gpu.clone());
    Ok(gpu)
}

pub(super) fn forget_device() {
    lock(&GPU).take();
}

/// Called each frame: a renderer on another device drops every surface's GPU frame.
pub(super) fn watch_device() {
    static SEEN: AtomicU64 = AtomicU64::new(0);
    let Some(lease) = gpui::renderer_metal_device() else {
        return;
    };
    let before = SEEN.swap(lease.generation, Ordering::Relaxed);
    if before != 0 && before != lease.generation {
        super::device_changed();
    }
}

pub(crate) struct Device(Arc<Gpu>);

impl Device {
    pub(crate) fn as_raw(&self) -> *mut c_void {
        Retained::as_ptr(&self.0.device).cast_mut().cast()
    }

    pub(crate) fn queue(&self) -> *mut c_void {
        Retained::as_ptr(&self.0.renderer_queue).cast_mut().cast()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.0.generation
    }
}

pub(super) fn device(state: &Mutex<State>) -> Result<Device, SurfaceError> {
    let gpu = gpu()?;
    lock(state).metal.handed = gpu.generation;
    Ok(Device(gpu))
}

pub(crate) struct Event(Arc<Queue>);

impl Event {
    pub(crate) fn as_raw(&self) -> *mut c_void {
        Retained::as_ptr(&self.0.event).cast_mut().cast()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.0.gpu.generation
    }

    pub(crate) fn signaled(&self) -> u64 {
        self.0.event.signaledValue()
    }

    pub(crate) fn wait(&self, value: u64, timeout: Duration) -> bool {
        if self.signaled() >= value {
            return true;
        }
        let (tx, rx) = mpsc::channel::<()>();
        let tx = Mutex::new(Some(tx));
        let listener = MTLSharedEventListener::new();
        let block = RcBlock::new(move |_: NonNull<SharedEvent>, _: u64| {
            if let Some(tx) = lock(&tx).take() {
                let _ = tx.send(());
            }
        });
        // SAFETY: Metal retains the listener and the block until it runs.
        unsafe {
            self.0
                .event
                .notifyListener_atValue_block(&listener, value, RcBlock::as_ptr(&block));
        }
        rx.recv_timeout(timeout).is_ok() || self.signaled() >= value
    }
}

pub(super) fn release_event(state: &Mutex<State>) -> Result<Event, SurfaceError> {
    let gpu = gpu()?;
    lock(state).metal.queue(&gpu).map(Event)
}

pub(crate) struct SlotTexture {
    texture: Retained<Texture>,
    /// A half-float slot's 8-bit picture and the release value its last mapping signals (0 until
    /// mapped for the frame in the slot).
    mapped: Option<(Retained<Texture>, Mutex<u64>)>,
}

// SAFETY: a Metal texture is thread-safe; the value is a mutex.
unsafe impl Send for SlotTexture {}
unsafe impl Sync for SlotTexture {}

impl SlotTexture {
    fn new(gpu: &Gpu, size: (u32, u32), format: SurfaceFormat) -> Result<Self, SurfaceError> {
        let texture = create_texture(gpu, size, pixel_format(format))?;
        let mapped = match format {
            SurfaceFormat::Bgra8 => None,
            SurfaceFormat::Rgba16Float => {
                rgba16f_pipeline(gpu)?;
                Some((
                    create_texture(gpu, size, MTLPixelFormat::BGRA8Unorm)?,
                    Mutex::new(0),
                ))
            }
        };
        Ok(Self { texture, mapped })
    }

    fn mapped(&self) -> Option<MutexGuard<'_, u64>> {
        self.mapped.as_ref().map(|(_, value)| lock(value))
    }
}

struct Slot {
    texture: Arc<SlotTexture>,
    size: (u32, u32),
    format: SurfaceFormat,
    generation: u64,
    written: u64,
}

#[derive(Default)]
pub(crate) struct SurfaceGpu {
    slots: Vec<Slot>,
    current: Option<usize>,
    queue: Option<Arc<Queue>>,
    /// The generation `metal_device` last handed out: a texture from another device after that one
    /// changed means the device changed, not that it is wrong.
    handed: u64,
}

// SAFETY: see `Gpu`.
unsafe impl Send for SurfaceGpu {}

/// A frame for the atlas: the texture, and the release value its copy (or its mapping, once
/// [`Ready::for_atlas`] made it) signals.
pub(crate) struct Ready {
    texture: Arc<SlotTexture>,
    queue: Arc<Queue>,
    value: u64,
    size: (u32, u32),
    mapped: bool,
}

// SAFETY: see `Gpu`.
unsafe impl Send for Ready {}
unsafe impl Sync for Ready {}

impl Ready {
    /// Whether the copy into the texture has run; until then the atlas must not read it.
    pub(crate) fn done(&self) -> bool {
        self.queue.event.signaledValue() >= self.value
    }

    /// What the atlas copies: the texture, or for a half-float frame its 8-bit mapping, encoded on
    /// the surface's queue the first time it is asked for. The atlas copy waits on [`Self::ready`].
    pub(crate) fn for_atlas(self) -> Result<Ready, SurfaceError> {
        let Some(mut mapped) = self.texture.mapped() else {
            return Ok(self);
        };
        if *mapped == 0 {
            *mapped = map(&self.queue, &self.texture, self.size)?;
        }
        let value = *mapped;
        drop(mapped);
        Ok(Ready {
            value,
            mapped: true,
            ..self
        })
    }

    /// A half-float frame, whose own texture an HDR window copies instead of its mapping.
    pub(crate) fn is_half_float(&self) -> bool {
        self.texture.mapped.is_some()
    }

    pub(crate) fn handle(&self) -> gpui::GpuTextureHandle {
        let texture = match (&self.texture.mapped, self.mapped) {
            (Some((mapped, _)), true) => mapped,
            _ => &self.texture.texture,
        };
        gpui::GpuTextureHandle(Retained::as_ptr(texture).cast_mut().cast())
    }

    /// The release event and the value the atlas copy waits for.
    pub(crate) fn ready(&self) -> (*mut c_void, u64) {
        (
            Retained::as_ptr(&self.queue.event).cast_mut().cast(),
            self.value,
        )
    }
}

impl SurfaceGpu {
    pub(crate) fn ready(&self) -> Option<Ready> {
        let slot = &self.slots[self.current?];
        Some(Ready {
            texture: slot.texture.clone(),
            queue: self.queue.clone()?,
            value: slot.written,
            size: slot.size,
            mapped: false,
        })
    }

    pub(crate) fn forget(&mut self) {
        self.slots.clear();
        self.current = None;
        self.queue = None;
    }

    fn queue(&mut self, gpu: &Arc<Gpu>) -> Result<Arc<Queue>, SurfaceError> {
        if let Some(queue) = self
            .queue
            .as_ref()
            .filter(|q| q.gpu.generation == gpu.generation)
        {
            return Ok(queue.clone());
        }
        if self.queue.is_some() {
            self.forget();
        }
        let queue = Queue::new(gpu)?;
        self.queue = Some(queue.clone());
        Ok(queue)
    }

    /// A slot neither shown as the frame nor still copied into or read by the atlas (both hold
    /// its `Arc` until their command buffer completes).
    fn free_slot(
        &mut self,
        gpu: &Gpu,
        size: (u32, u32),
        format: SurfaceFormat,
    ) -> Result<usize, SurfaceError> {
        if self.slots.iter().any(|s| s.generation != gpu.generation) {
            self.slots.clear();
            self.current = None;
        }
        let free = |(i, s): &(usize, &Slot)| {
            Some(*i) != self.current && Arc::strong_count(&s.texture) == 1
        };
        let fits = |s: &Slot| s.size == size && s.format == format;
        let pick = self
            .slots
            .iter()
            .enumerate()
            .filter(free)
            .min_by_key(|(_, s)| !fits(s))
            .map(|(i, _)| i);
        let slot = || -> Result<Slot, SurfaceError> {
            Ok(Slot {
                texture: Arc::new(SlotTexture::new(gpu, size, format)?),
                size,
                format,
                generation: gpu.generation,
                written: 0,
            })
        };
        match pick {
            Some(i) if fits(&self.slots[i]) => Ok(i),
            Some(i) => {
                self.slots[i] = slot()?;
                Ok(i)
            }
            None if self.slots.len() < MAX_SLOTS => {
                self.slots.push(slot()?);
                Ok(self.slots.len() - 1)
            }
            None => Err(SurfaceError::Busy),
        }
    }
}

fn pixel_format(format: SurfaceFormat) -> MTLPixelFormat {
    match format {
        SurfaceFormat::Bgra8 => MTLPixelFormat::BGRA8Unorm,
        SurfaceFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
    }
}

fn descriptor(
    format: MTLPixelFormat,
    (width, height): (u32, u32),
) -> Retained<MTLTextureDescriptor> {
    let desc = MTLTextureDescriptor::new();
    desc.setTextureType(MTLTextureType::Type2D);
    desc.setPixelFormat(format);
    // SAFETY: both sides were checked against the renderer's texture limit.
    unsafe {
        desc.setWidth(width as usize);
        desc.setHeight(height as usize);
    }
    desc.setUsage(MTLTextureUsage::ShaderRead);
    desc
}

fn create_texture(
    gpu: &Gpu,
    size: (u32, u32),
    format: MTLPixelFormat,
) -> Result<Retained<Texture>, SurfaceError> {
    let desc = descriptor(format, size);
    desc.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
    desc.setStorageMode(MTLStorageMode::Private);
    gpu.device
        .newTextureWithDescriptor(&desc)
        .ok_or_else(|| platform("creating the surface texture"))
}

/// The texture's size and format, and the texture itself or a `BGRA8Unorm` view of an `_sRGB` one
/// (a copy keeps the format).
fn describe(
    texture: &Texture,
) -> Result<(Retained<Texture>, (u32, u32), SurfaceFormat), SurfaceError> {
    let format = texture.pixelFormat();
    let surface_format = match format {
        MTLPixelFormat::BGRA8Unorm | MTLPixelFormat::BGRA8Unorm_sRGB => SurfaceFormat::Bgra8,
        MTLPixelFormat::RGBA16Float => SurfaceFormat::Rgba16Float,
        _ => {
            return Err(SurfaceError::Format(format!(
                "a texture in MTLPixelFormat {}: a surface takes a 2D BGRA8Unorm (or \
                 BGRA8Unorm_sRGB) texture, or RGBA16Float on a surface set to Rgba16Float",
                format.0
            )))
        }
    };
    if texture.textureType() != MTLTextureType::Type2D {
        return Err(SurfaceError::Format(
            "a texture that is not 2D: a surface takes a 2D texture".into(),
        ));
    }
    if texture.sampleCount() != 1 {
        return Err(SurfaceError::Format(
            "a multisampled texture: resolve it to a single-sampled texture first".into(),
        ));
    }
    let size = (
        u32::try_from(texture.width()).unwrap_or(u32::MAX),
        u32::try_from(texture.height()).unwrap_or(u32::MAX),
    );
    check_size(size.0, size.1)?;
    let texture = if format != MTLPixelFormat::BGRA8Unorm_sRGB {
        texture.retain()
    } else {
        texture
            .newTextureViewWithPixelFormat(MTLPixelFormat::BGRA8Unorm)
            .ok_or_else(|| {
                SurfaceError::Format(
                    "a BGRA8Unorm_sRGB texture Metal gives no BGRA8Unorm view of: create it with \
                     MTLTextureUsagePixelFormatView"
                        .into(),
                )
            })?
    };
    Ok((texture, size, surface_format))
}

const RGBA16F_KERNEL: &str = r#"
#include <metal_stdlib>
using namespace metal;

kernel void rgba16f_to_bgra(texture2d<float, access::read> frame [[texture(0)]],
                            texture2d<float, access::write> out [[texture(1)]],
                            uint2 at [[thread_position_in_grid]]) {
    if (at.x >= out.get_width() || at.y >= out.get_height()) return;
    float4 texel = frame.read(at);
    float3 c = saturate(texel.rgb);
    float3 srgb = select(1.055 * precise::pow(c, float3(1.0 / 2.4)) - 0.055, c * 12.92,
                         c < 0.0031308);
    out.write(float4(srgb, saturate(texel.a)), at);
}
"#;

fn rgba16f_pipeline(gpu: &Gpu) -> Result<Pipeline, SurfaceError> {
    kernel(&gpu.rgba16f, &gpu.device, RGBA16F_KERNEL, "rgba16f_to_bgra")
}

/// Maps a half-float slot into its 8-bit texture on the surface's queue, after the copy into it.
/// Returns the release value the mapping signals.
fn map(queue: &Queue, slot: &Arc<SlotTexture>, size: (u32, u32)) -> Result<u64, SurfaceError> {
    let (mapped, _) = slot.mapped.as_ref().expect("a half-float slot");
    let pipeline = rgba16f_pipeline(&queue.gpu)?;
    let mut next = lock(&queue.next);
    let command = queue
        .queue
        .commandBuffer()
        .ok_or_else(|| platform("creating a surface command buffer"))?;
    let encoder = command
        .computeCommandEncoder()
        .ok_or_else(|| platform("creating a compute encoder"))?;
    encoder.setComputePipelineState(&pipeline);
    let width = pipeline.threadExecutionWidth().max(1);
    let height = (pipeline.maxTotalThreadsPerThreadgroup() / width).clamp(1, 16);
    // SAFETY: the kernel reads texture 0 and writes texture 1, both `size`; the grid is `size`.
    unsafe {
        encoder.setTexture_atIndex(Some(&slot.texture), 0);
        encoder.setTexture_atIndex(Some(mapped), 1);
        encoder.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: size.0 as usize,
                height: size.1 as usize,
                depth: 1,
            },
            MTLSize {
                width,
                height,
                depth: 1,
            },
        );
    }
    encoder.endEncoding();
    let value = *next + 1;
    command.encodeSignalEvent_value(ProtocolObject::from_ref(&*queue.event), value);
    let held = Mutex::new(Some(slot.clone()));
    let done = RcBlock::new(move |_: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
        lock(&held).take();
    });
    // SAFETY: Metal retains the block until it has run.
    unsafe { command.addCompletedHandler(RcBlock::as_ptr(&done)) };
    command.commit();
    *next = value;
    Ok(value)
}
/// What a present copies into a surface texture.
enum Source<'a> {
    /// Copied as it is: BGRA8 or half-float.
    Texture(&'a Texture, SurfaceFormat),
    /// Bi-planar 4:2:0 luma and chroma planes, converted with `[y offset, y scale, c offset,
    /// c scale, r←cr, g←cb, g←cr, b←cb]`.
    Nv12 {
        luma: &'a Texture,
        chroma: &'a Texture,
        coefficients: [f32; 8],
    },
}

/// Keeps what a copy reads until its command buffer completes.
struct Held(#[allow(dead_code)] Vec<Box<dyn Any>>);

/// Copies `source` into a free surface texture after `wait`, signals the release event (and
/// `signal`), and makes the texture the frame. Returns the release value.
fn copy_in(
    gpu: &Arc<Gpu>,
    state: &mut State,
    source: Source<'_>,
    size: (u32, u32),
    wait: Option<(&SharedEvent, u64)>,
    signal: Option<(&SharedEvent, u64)>,
    mut held: Vec<Box<dyn Any>>,
) -> Result<(u64, u64), SurfaceError> {
    let pipeline = match &source {
        Source::Nv12 { .. } => Some(nv12_pipeline(gpu)?),
        Source::Texture(..) => None,
    };
    let format = match &source {
        Source::Texture(_, format) => *format,
        Source::Nv12 { .. } => SurfaceFormat::Bgra8,
    };
    check_format(state.format, format)?;
    let queue = state.metal.queue(gpu)?;
    let index = state.metal.free_slot(gpu, size, format)?;
    let target = state.metal.slots[index].texture.clone();
    if let Some(mut mapped) = target.mapped() {
        *mapped = 0;
    }
    let mut next = lock(&queue.next);
    let command = queue
        .queue
        .commandBuffer()
        .ok_or_else(|| platform("creating a surface command buffer"))?;
    if let Some((event, value)) = wait {
        command.encodeWaitForEvent_value(ProtocolObject::from_ref(event), value);
    }
    let whole = MTLSize {
        width: size.0 as usize,
        height: size.1 as usize,
        depth: 1,
    };
    match source {
        Source::Texture(texture, _) => {
            let encoder = command
                .blitCommandEncoder()
                .ok_or_else(|| platform("creating a blit encoder"))?;
            let origin = MTLOrigin { x: 0, y: 0, z: 0 };
            // SAFETY: both textures have the same pixel format, are 2D and at least `size`.
            unsafe {
                encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                    texture, 0, 0, origin, whole, &target.texture, 0, 0, origin,
                );
            }
            encoder.endEncoding();
        }
        Source::Nv12 {
            luma,
            chroma,
            coefficients,
        } => {
            let pipeline = pipeline.expect("made above");
            let encoder = command
                .computeCommandEncoder()
                .ok_or_else(|| platform("creating a compute encoder"))?;
            encoder.setComputePipelineState(&pipeline);
            let width = pipeline.threadExecutionWidth().max(1);
            let height = (pipeline.maxTotalThreadsPerThreadgroup() / width).clamp(1, 16);
            // SAFETY: the kernel reads textures 0 and 1 and the 32 bytes at buffer 0, and writes
            // texture 2, which is `size`; the grid is `size`.
            unsafe {
                encoder.setTexture_atIndex(Some(luma), 0);
                encoder.setTexture_atIndex(Some(chroma), 1);
                encoder.setTexture_atIndex(Some(&target.texture), 2);
                encoder.setBytes_length_atIndex(
                    NonNull::from(&coefficients).cast(),
                    std::mem::size_of_val(&coefficients),
                    0,
                );
                encoder.dispatchThreads_threadsPerThreadgroup(
                    whole,
                    MTLSize {
                        width,
                        height,
                        depth: 1,
                    },
                );
            }
            encoder.endEncoding();
        }
    }
    let value = *next + 1;
    command.encodeSignalEvent_value(ProtocolObject::from_ref(&*queue.event), value);
    if let Some((event, value)) = signal {
        command.encodeSignalEvent_value(ProtocolObject::from_ref(event), value);
    }
    for (event, _) in wait.into_iter().chain(signal) {
        held.push(Box::new(event.retain()));
    }
    held.push(Box::new(target.clone()));
    let held = Mutex::new(Some(Held(held)));
    let done = RcBlock::new(move |_: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
        lock(&held).take();
    });
    // SAFETY: Metal retains the block until it has run.
    unsafe { command.addCompletedHandler(RcBlock::as_ptr(&done)) };
    command.commit();
    *next = value;
    drop(next);
    state.metal.slots[index].written = value;
    state.metal.current = Some(index);
    let frame = state.put(Frame::Metal {
        width: size.0,
        height: size.1,
    });
    Ok((frame, value))
}

const NV12_KERNEL: &str = r#"
#include <metal_stdlib>
using namespace metal;

kernel void nv12_to_bgra(texture2d<float, access::read> luma [[texture(0)]],
                         texture2d<float, access::sample> chroma [[texture(1)]],
                         texture2d<float, access::write> out [[texture(2)]],
                         constant float4 *k [[buffer(0)]],
                         uint2 at [[thread_position_in_grid]]) {
    if (at.x >= out.get_width() || at.y >= out.get_height()) return;
    constexpr sampler linear(filter::linear, address::clamp_to_edge, coord::normalized);
    float2 uv = (float2(at) + 0.5) / float2(out.get_width(), out.get_height());
    float y = (luma.read(at).r - k[0].x) * k[0].y;
    float2 c = (chroma.sample(linear, uv).rg - k[0].z) * k[0].w;
    float3 rgb = float3(y + k[1].x * c.y, y - k[1].y * c.x - k[1].z * c.y, y + k[1].w * c.x);
    out.write(float4(saturate(rgb), 1.0), at);
}
"#;

type Pipeline = Retained<ProtocolObject<dyn MTLComputePipelineState>>;

/// Compiles a kernel on first use; a failure is kept and returned each time.
fn kernel(
    cell: &OnceLock<Result<Pipeline, String>>,
    device: &ProtocolObject<dyn MTLDevice>,
    source: &str,
    entry: &str,
) -> Result<Pipeline, SurfaceError> {
    cell.get_or_init(|| {
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(source), None)
            .map_err(|e| format!("compiling the {entry} kernel: {e}"))?;
        let function = library
            .newFunctionWithName(&NSString::from_str(entry))
            .ok_or_else(|| format!("the {entry} kernel has no entry point"))?;
        device
            .newComputePipelineStateWithFunction_error(&function)
            .map_err(|e| format!("building the {entry} pipeline: {e}"))
    })
    .clone()
    .map_err(SurfaceError::Platform)
}

fn nv12_pipeline(gpu: &Gpu) -> Result<Pipeline, SurfaceError> {
    kernel(&gpu.nv12, &gpu.device, NV12_KERNEL, "nv12_to_bgra")
}

/// The device a texture was made on is the renderer's, or the producer has a stale one.
fn check_device(state: &Mutex<State>, gpu: &Gpu, texture: &Texture) -> Result<(), SurfaceError> {
    if std::ptr::eq(
        Retained::as_ptr(&texture.device()),
        Retained::as_ptr(&gpu.device),
    ) {
        return Ok(());
    }
    let handed = lock(state).metal.handed;
    Err(if handed != 0 && handed != gpu.generation {
        SurfaceError::DeviceChanged
    } else {
        SurfaceError::WrongDevice
    })
}

/// # Safety
/// `raw` is null or a live `id<MTLTexture>`; `ready` is a live `id<MTLSharedEvent>` and a value.
pub(super) unsafe fn present(
    state: &Mutex<State>,
    raw: *mut c_void,
    ready: Option<(*mut c_void, u64)>,
) -> Result<(u64, u64), SurfaceError> {
    // SAFETY: the caller's.
    let texture = unsafe { Retained::retain(raw.cast::<Texture>()) }
        .ok_or_else(|| SurfaceError::Size("a null texture".into()))?;
    let wait = match ready {
        Some((event, value)) => Some((
            // SAFETY: the caller's.
            unsafe { Retained::retain(event.cast::<SharedEvent>()) }
                .ok_or_else(|| SurfaceError::Size("a null shared event".into()))?,
            value,
        )),
        None => None,
    };
    let gpu = gpu()?;
    check_device(state, &gpu, &texture)?;
    let (view, size, format) = describe(&texture)?;
    let mut state = lock(state);
    copy_in(
        &gpu,
        &mut state,
        Source::Texture(&view, format),
        size,
        wait.as_ref().map(|(e, v)| (&**e, *v)),
        None,
        vec![Box::new(view.clone())],
    )
}

fn iosurface_texture(
    gpu: &Gpu,
    surface: &IOSurfaceRef,
    format: MTLPixelFormat,
    plane: usize,
    size: (u32, u32),
) -> Result<Retained<Texture>, SurfaceError> {
    let desc = descriptor(format, size);
    gpu.device
        .newTextureWithDescriptor_iosurface_plane(&desc, surface, plane)
        .ok_or_else(|| platform("making a Metal texture of the IOSurface"))
}

/// An IOSurface's size and format: 'BGRA' or 'RGhA'.
fn iosurface_format(surface: &IOSurfaceRef) -> Result<((u32, u32), SurfaceFormat), SurfaceError> {
    let format = match surface.pixel_format() {
        BGRA => SurfaceFormat::Bgra8,
        RGHA => SurfaceFormat::Rgba16Float,
        other => {
            return Err(SurfaceError::Format(format!(
                "an IOSurface of pixel format {}: a surface takes 'BGRA' \
                 (kCVPixelFormatType_32BGRA), or 'RGhA' (kCVPixelFormatType_64RGBAHalf) on a \
                 surface set to Rgba16Float; present 420v/420f video as a CVPixelBuffer",
                fourcc(other)
            )))
        }
    };
    let size = (
        u32::try_from(surface.width()).unwrap_or(u32::MAX),
        u32::try_from(surface.height()).unwrap_or(u32::MAX),
    );
    check_size(size.0, size.1)?;
    Ok((size, format))
}

fn fourcc(code: u32) -> String {
    let bytes = code.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        format!("{code:#x}")
    }
}

pub(crate) struct Opened {
    surface: CFRetained<IOSurfaceRef>,
    texture: Retained<Texture>,
    event: Option<Retained<SharedEvent>>,
    generation: u64,
    pub(super) size: (u32, u32),
    pub(super) format: SurfaceFormat,
}

// SAFETY: IOSurfaces and Metal objects are thread-safe.
unsafe impl Send for Opened {}

impl Opened {
    pub(super) fn fenced(&self) -> bool {
        self.event.is_some()
    }

    /// # Safety
    /// `event` is null or a live `id<MTLSharedEvent>`.
    pub(super) unsafe fn set_event(&mut self, event: *mut c_void) -> Result<(), SurfaceError> {
        // SAFETY: the caller's.
        self.event = Some(
            unsafe { Retained::retain(event.cast::<SharedEvent>()) }
                .ok_or_else(|| SurfaceError::Size("a null shared event".into()))?,
        );
        Ok(())
    }

    /// # Safety
    /// `handle` is null or a live `MTLSharedEventHandle`.
    pub(super) unsafe fn set_event_handle(
        &mut self,
        handle: *mut c_void,
    ) -> Result<(), SurfaceError> {
        // SAFETY: the caller's.
        let handle = unsafe { Retained::retain(handle.cast::<MTLSharedEventHandle>()) }
            .ok_or_else(|| SurfaceError::Size("a null MTLSharedEventHandle".into()))?;
        let gpu = gpu()?;
        self.event = Some(
            gpu.device
                .newSharedEventWithHandle(&handle)
                .ok_or_else(|| platform("opening the MTLSharedEventHandle"))?,
        );
        Ok(())
    }
}

pub(super) fn open(surface: CFRetained<IOSurfaceRef>) -> Result<Opened, SurfaceError> {
    let (size, format) = iosurface_format(&surface)?;
    let gpu = gpu()?;
    let texture = iosurface_texture(&gpu, &surface, pixel_format(format), 0, size)?;
    Ok(Opened {
        surface,
        texture,
        event: None,
        generation: gpu.generation,
        size,
        format,
    })
}

/// # Safety
/// `surface` is null or a live `IOSurfaceRef`.
pub(super) unsafe fn open_raw(surface: *mut c_void) -> Result<Opened, SurfaceError> {
    let surface = NonNull::new(surface.cast::<IOSurfaceRef>())
        .ok_or_else(|| SurfaceError::Size("a null IOSurfaceRef".into()))?;
    // SAFETY: the caller's; retaining keeps it alive for the texture's life.
    open(unsafe { CFRetained::retain(surface) })
}

pub(super) fn open_mach_port(port: u32) -> Result<Opened, SurfaceError> {
    let surface = IOSurfaceRef::lookup_from_mach_port(port).ok_or_else(|| {
        SurfaceError::Platform(format!(
            "mach port {port} names no IOSurface in this process (IOSurfaceCreateMachPort makes \
             one; send it with its send right)"
        ))
    })?;
    open(surface)
}

pub(super) fn present_shared(
    state: &Mutex<State>,
    opened: &Opened,
    values: Option<(u64, u64)>,
) -> Result<u64, SurfaceError> {
    let gpu = gpu()?;
    if gpu.generation != opened.generation {
        return Err(SurfaceError::DeviceChanged);
    }
    let (wait, signal) = match (values, opened.event.as_deref()) {
        (Some((wait, release)), Some(event)) => (Some((event, wait)), Some((event, release))),
        _ => (None, None),
    };
    let mut state = lock(state);
    copy_in(
        &gpu,
        &mut state,
        Source::Texture(&opened.texture, opened.format),
        opened.size,
        wait,
        signal,
        vec![Box::new(opened.surface.clone())],
    )
    .map(|(frame, _)| frame)
}

/// `[y offset, y scale, c offset, c scale]` for 8-bit video or full range, then the matrix.
fn coefficients(full_range: bool, matrix: Option<String>) -> [f32; 8] {
    let range = if full_range {
        [0.0, 1.0, 128.0 / 255.0, 1.0]
    } else {
        [16.0 / 255.0, 255.0 / 219.0, 128.0 / 255.0, 255.0 / 224.0]
    };
    let (kr, kb): (f32, f32) = match matrix.as_deref() {
        Some("ITU_R_601_4" | "SMPTE_240M_1995") => (0.299, 0.114),
        Some("ITU_R_2020") => (0.2627, 0.0593),
        _ => (0.2126, 0.0722),
    };
    let kg = 1.0 - kr - kb;
    let r_cr = 2.0 * (1.0 - kr);
    let b_cb = 2.0 * (1.0 - kb);
    [
        range[0],
        range[1],
        range[2],
        range[3],
        r_cr,
        b_cb * kb / kg,
        r_cr * kr / kg,
        b_cb,
    ]
}

fn ycbcr_matrix(buffer: &CVPixelBuffer) -> Option<String> {
    let key = CFString::from_static_str("CVImageBufferYCbCrMatrix");
    // SAFETY: a live buffer and key; the mode out-pointer may be null.
    let value = unsafe { buffer.attachment(&key, std::ptr::null_mut()) }?;
    Some(value.downcast_ref::<CFString>()?.to_string())
}

/// A CPU-only buffer, for the caller to present as a CPU frame: BGRA bytes, or for
/// [`SurfaceFormat::Rgba16Float`] native-endian halves.
pub(super) struct Cpu<'a> {
    pub(super) pixels: &'a [u8],
    pub(super) format: SurfaceFormat,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) stride: usize,
}

/// # Safety
/// `raw` is null or a live `CVPixelBufferRef`.
pub(super) unsafe fn present_pixel_buffer(
    state: &Mutex<State>,
    raw: *mut c_void,
    cpu: impl FnOnce(Cpu<'_>) -> Result<u64, SurfaceError>,
) -> Result<u64, SurfaceError> {
    let buffer = NonNull::new(raw.cast::<CVPixelBuffer>())
        .ok_or_else(|| SurfaceError::Size("a null CVPixelBufferRef".into()))?;
    // SAFETY: the caller's; retained until the copy completes.
    let buffer: CFRetained<CVPixelBuffer> = unsafe { CFRetained::retain(buffer) };
    let format = CVPixelBufferGetPixelFormatType(&buffer);
    let full_range = match format {
        BGRA | RGHA | NV12_VIDEO => false,
        NV12_FULL => true,
        _ => {
            return Err(SurfaceError::Format(format!(
                "a CVPixelBuffer of pixel format {}: a surface takes BGRA (32BGRA), 4:2:0 \
                 bi-planar (420v, 420f), or RGhA (64RGBAHalf) on a surface set to Rgba16Float",
                fourcc(format)
            )))
        }
    };
    let surface_format = if format == RGHA {
        SurfaceFormat::Rgba16Float
    } else {
        SurfaceFormat::Bgra8
    };
    let size = (
        u32::try_from(CVPixelBufferGetWidth(&buffer)).unwrap_or(u32::MAX),
        u32::try_from(CVPixelBufferGetHeight(&buffer)).unwrap_or(u32::MAX),
    );
    check_size(size.0, size.1)?;
    let Some(surface) = CVPixelBufferGetIOSurface(Some(&buffer)) else {
        if format != BGRA && format != RGHA {
            return Err(SurfaceError::Format(
                "a 4:2:0 CVPixelBuffer without an IOSurface: create it with \
                 kCVPixelBufferIOSurfacePropertiesKey (VideoToolbox and AVFoundation do)"
                    .into(),
            ));
        }
        // SAFETY: locked read-only around the read, the base address covers `stride * height`.
        unsafe {
            if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly)
                != kCVReturnSuccess
            {
                return Err(platform("locking the CVPixelBuffer"));
            }
            let base = CVPixelBufferGetBaseAddress(&buffer);
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            let result = if base.is_null() {
                Err(platform("reading the CVPixelBuffer"))
            } else {
                cpu(Cpu {
                    pixels: std::slice::from_raw_parts(base.cast::<u8>(), stride * size.1 as usize),
                    format: surface_format,
                    width: size.0,
                    height: size.1,
                    stride,
                })
            };
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly);
            return result;
        }
    };
    let gpu = gpu()?;
    let mut held: Vec<Box<dyn Any>> = Vec::new();
    let value = if format == BGRA || format == RGHA {
        let pixels = pixel_format(surface_format);
        let texture = iosurface_texture(&gpu, &surface, pixels, 0, size)?;
        held.push(Box::new(buffer.clone()));
        let mut state = lock(state);
        copy_in(
            &gpu,
            &mut state,
            Source::Texture(&texture, surface_format),
            size,
            None,
            None,
            held,
        )
    } else {
        let chroma_size = (size.0.div_ceil(2), size.1.div_ceil(2));
        let luma = iosurface_texture(&gpu, &surface, MTLPixelFormat::R8Unorm, 0, size)?;
        let chroma = iosurface_texture(&gpu, &surface, MTLPixelFormat::RG8Unorm, 1, chroma_size)?;
        let coefficients = coefficients(full_range, ycbcr_matrix(&buffer));
        held.push(Box::new(buffer.clone()));
        let mut state = lock(state);
        copy_in(
            &gpu,
            &mut state,
            Source::Nv12 {
                luma: &luma,
                chroma: &chroma,
                coefficients,
            },
            size,
            None,
            None,
            held,
        )
    };
    value.map(|(frame, _)| frame)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use objc2_core_foundation::{CFDictionary, CFType};
    use objc2_core_video::{
        kCVPixelBufferIOSurfacePropertiesKey, CVPixelBufferCreate,
        CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
        CVPixelBufferGetHeightOfPlane, CVPixelBufferGetWidthOfPlane,
    };
    use objc2_metal::{
        MTLBuffer, MTLCreateSystemDefaultDevice, MTLRegion, MTLResourceOptions, MTLStorageMode,
    };

    use super::*;
    use crate::surface::color::tests::{f32_to_half, ramp};
    use crate::surface::{surface, SharedTexture, SurfaceHandle};

    const W: u32 = 64;
    const H: u32 = 8;

    /// The system device, published as the renderer's the way a window does; `None` without a GPU.
    fn renderer() -> Option<Retained<ProtocolObject<dyn MTLDevice>>> {
        static ONCE: OnceLock<()> = OnceLock::new();
        let device = MTLCreateSystemDefaultDevice()?;
        ONCE.get_or_init(|| {
            struct Kept(
                #[allow(dead_code)] Retained<ProtocolObject<dyn MTLDevice>>,
                #[allow(dead_code)] Retained<ProtocolObject<dyn MTLCommandQueue>>,
            );
            unsafe impl Send for Kept {}
            unsafe impl Sync for Kept {}
            let queue = device.newCommandQueue().unwrap();
            gpui::publish_renderer_metal_device(
                raw(&*device),
                raw(&*queue),
                Arc::new(Kept(device.clone(), queue)),
            );
        });
        Some(device)
    }

    fn raw<T: ?Sized>(object: &T) -> *mut c_void {
        (object as *const T).cast_mut().cast()
    }

    fn filled(device: &ProtocolObject<dyn MTLDevice>, bgra: [u8; 4]) -> Retained<Texture> {
        let bytes = bgra.repeat((W * H) as usize);
        written(device, MTLPixelFormat::BGRA8Unorm, (W, H), &bytes)
    }

    /// A texture holding `bytes`, rows packed.
    fn written(
        device: &ProtocolObject<dyn MTLDevice>,
        format: MTLPixelFormat,
        (width, height): (u32, u32),
        bytes: &[u8],
    ) -> Retained<Texture> {
        let desc = descriptor(format, (width, height));
        desc.setStorageMode(if device.hasUnifiedMemory() {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        let texture = device.newTextureWithDescriptor(&desc).unwrap();
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: width as usize,
                height: height as usize,
                depth: 1,
            },
        };
        unsafe {
            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region,
                0,
                NonNull::from(bytes).cast(),
                bytes.len() / height as usize,
            );
        }
        texture
    }

    fn halves(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_ne_bytes()).collect()
    }

    fn pixel_buffer(format: u32, iosurface: bool) -> CFRetained<CVPixelBuffer> {
        let key = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
        let empty = CFDictionary::<CFType, CFType>::empty();
        let attributes = CFDictionary::from_slices(&[key], &[&*empty]);
        let mut buffer = std::ptr::null_mut();
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                W as usize,
                H as usize,
                format,
                iosurface.then(|| attributes.as_opaque()),
                NonNull::from(&mut buffer),
            )
        };
        assert_eq!(status, kCVReturnSuccess);
        unsafe { CFRetained::from_raw(NonNull::new(buffer).unwrap()) }
    }

    fn fill(buffer: &CVPixelBuffer, plane: usize, texel: &[u8]) {
        unsafe {
            CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
            let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane);
            let width = CVPixelBufferGetWidthOfPlane(buffer, plane);
            for y in 0..CVPixelBufferGetHeightOfPlane(buffer, plane) {
                let row = std::slice::from_raw_parts_mut(base.add(y * stride), width * texel.len());
                for px in row.chunks_exact_mut(texel.len()) {
                    px.copy_from_slice(texel);
                }
            }
            CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty());
        }
    }

    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < until, "{what} never happened");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// What the atlas would copy for the surface's frame, read back once it is written.
    fn shown(surface: &SurfaceHandle) -> Vec<[u8; 4]> {
        let ready = surface.lock_state().metal.ready().expect("a Metal frame");
        eventually("the copy", || ready.done());
        let ready = ready.for_atlas().unwrap();
        eventually("the mapping", || ready.done());
        let texture = match (&ready.texture.mapped, ready.mapped) {
            (Some((mapped, _)), true) => mapped,
            _ => &ready.texture.texture,
        };
        assert_eq!(texture.pixelFormat(), MTLPixelFormat::BGRA8Unorm);
        let (w, h) = (texture.width(), texture.height());
        let device = &ready.queue.gpu.device;
        let buffer = device
            .newBufferWithLength_options(w * h * 4, MTLResourceOptions::StorageModeShared)
            .unwrap();
        let command = device.newCommandQueue().unwrap().commandBuffer().unwrap();
        let encoder = command.blitCommandEncoder().unwrap();
        unsafe {
            encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
                texture,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
                MTLSize { width: w, height: h, depth: 1 },
                &buffer,
                0,
                w * 4,
                w * h * 4,
            );
        }
        encoder.endEncoding();
        command.commit();
        command.waitUntilCompleted();
        let bytes = unsafe {
            std::slice::from_raw_parts(buffer.contents().cast::<u8>().as_ptr(), w * h * 4)
        };
        bytes
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect()
    }

    fn all_near(pixels: &[[u8; 4]], want: [u8; 4], by: u8) -> bool {
        pixels
            .iter()
            .all(|p| p.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= by))
    }

    #[test]
    fn a_texture_is_copied_once_its_event_fires_and_then_released() {
        let Some(device) = renderer() else { return };
        let s = surface("tests:metal-wait");
        let source = filled(&device, [10, 20, 30, 255]);
        let event = device.newSharedEvent().unwrap();
        let released = unsafe { s.present_metal_texture(raw(&*source), Some((raw(&*event), 1))) }
            .unwrap()
            .release;
        let ready = s.lock_state().metal.ready().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !ready.done(),
            "the copy waits for the producer's event on the GPU"
        );
        event.setSignaledValue(1);
        assert!(s
            .release_event()
            .unwrap()
            .wait(released, Duration::from_secs(5)));
        assert!(all_near(&shown(&s), [10, 20, 30, 255], 0));
    }

    #[test]
    fn an_event_that_never_fires_stalls_only_its_own_surface() {
        let Some(device) = renderer() else { return };
        let stalled = surface("tests:metal-stalled");
        let other = surface("tests:metal-other");
        let never = device.newSharedEvent().unwrap();
        let a = filled(&device, [1, 2, 3, 255]);
        unsafe { stalled.present_metal_texture(raw(&*a), Some((raw(&*never), 1))) }.unwrap();
        let b = filled(&device, [4, 5, 6, 255]);
        let released = unsafe { other.present_metal_texture(raw(&*b), None) }
            .unwrap()
            .release;
        assert!(other
            .release_event()
            .unwrap()
            .wait(released, Duration::from_secs(5)));
        assert!(!stalled.lock_state().metal.ready().unwrap().done());
        never.setSignaledValue(1);
    }

    #[test]
    fn a_fifth_frame_in_flight_is_refused_until_one_is_released() {
        let Some(device) = renderer() else { return };
        let s = surface("tests:metal-busy");
        let source = filled(&device, [1, 2, 3, 255]);
        let event = device.newSharedEvent().unwrap();
        let present = || unsafe { s.present_metal_texture(raw(&*source), Some((raw(&*event), 1))) };
        for _ in 0..MAX_SLOTS {
            present().unwrap();
        }
        assert_eq!(present(), Err(SurfaceError::Busy));
        event.setSignaledValue(1);
        eventually("a free slot", || present().is_ok());
        assert!(all_near(&shown(&s), [1, 2, 3, 255], 0));
    }

    #[test]
    fn a_shared_iosurface_waits_for_its_event_and_signals_the_release_value() {
        let Some(device) = renderer() else { return };
        let s = surface("tests:metal-iosurface");
        let buffer = pixel_buffer(BGRA, true);
        fill(&buffer, 0, &[200, 100, 50, 255]);
        let iosurface = CVPixelBufferGetIOSurface(Some(&buffer)).unwrap();
        let event = device.newSharedEvent().unwrap();
        let shared = unsafe { SharedTexture::from_iosurface(raw(&*iosurface)) }
            .and_then(|t| unsafe { t.with_shared_event(raw(&*event)) })
            .unwrap();
        assert_eq!(shared.size(), (W, H));
        assert!(matches!(
            s.present_shared(&shared),
            Err(SurfaceError::Format(_))
        ));
        s.present_shared_fenced(&shared, 1, 2).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(event.signaledValue(), 0);
        event.setSignaledValue(1);
        eventually("the release value", || event.signaledValue() == 2);
        assert!(all_near(&shown(&s), [200, 100, 50, 255], 0));
    }

    #[test]
    fn pixel_buffers_convert_420v_and_copy_bgra() {
        if renderer().is_none() {
            return;
        }
        let s = surface("tests:metal-cv");
        // BT.709 video-range red.
        let nv12 = pixel_buffer(NV12_VIDEO, true);
        fill(&nv12, 0, &[63]);
        fill(&nv12, 1, &[102, 240]);
        unsafe { s.present_cv_pixel_buffer(raw(&*nv12)) }.unwrap();
        assert!(all_near(&shown(&s), [0, 0, 255, 255], 2));

        let bgra = pixel_buffer(BGRA, true);
        fill(&bgra, 0, &[9, 8, 7, 255]);
        unsafe { s.present_cv_pixel_buffer(raw(&*bgra)) }.unwrap();
        assert!(all_near(&shown(&s), [9, 8, 7, 255], 0));

        let cpu_only = pixel_buffer(BGRA, false);
        fill(&cpu_only, 0, &[4, 5, 6, 128]);
        unsafe { s.present_cv_pixel_buffer(raw(&*cpu_only)) }.unwrap();
        let frame = s.lock_state().frame.clone();
        match frame {
            Some(Frame::Cpu { bgra, .. }) => assert!(bgra.chunks(4).all(|p| p == [4, 5, 6, 128])),
            _ => panic!("a buffer with no IOSurface is a CPU frame"),
        }
    }

    #[test]
    fn a_half_float_texture_shows_every_srgb_code_unchanged_and_clamps_above_one() {
        let Some(device) = renderer() else { return };
        let s = surface("tests:metal-rgba16f");
        s.set_format(SurfaceFormat::Rgba16Float);
        let h = f32_to_half;
        let mut texels = ramp();
        texels.extend([h(8.0), h(1.5), h(-0.25), h(2.0)]);
        let source = written(
            &device,
            MTLPixelFormat::RGBA16Float,
            (257, 1),
            &halves(&texels),
        );
        unsafe { s.present_metal_texture(raw(&*source), None) }.unwrap();
        let pixels = shown(&s);
        for (code, px) in pixels[..256].iter().enumerate() {
            assert_eq!(px, &[code as u8; 4], "code {code}");
        }
        assert_eq!(pixels[256], [0, 255, 255, 255]);
    }

    #[test]
    fn rgha_pixel_buffers_and_iosurfaces_are_mapped() {
        if renderer().is_none() {
            return;
        }
        let s = surface("tests:metal-rgha");
        s.set_format(SurfaceFormat::Rgba16Float);
        let h = f32_to_half;
        let texel = halves(&[h(0.2159), h(1.0), h(4.0), h(1.0)]);
        let want = [255, 255, 128, 255];

        let buffer = pixel_buffer(RGHA, true);
        fill(&buffer, 0, &texel);
        unsafe { s.present_cv_pixel_buffer(raw(&*buffer)) }.unwrap();
        assert!(all_near(&shown(&s), want, 0));

        let iosurface = CVPixelBufferGetIOSurface(Some(&buffer)).unwrap();
        let shared = unsafe { SharedTexture::from_iosurface(raw(&*iosurface)) }.unwrap();
        assert_eq!(shared.format(), SurfaceFormat::Rgba16Float);
        s.present_shared(&shared).unwrap();
        assert!(all_near(&shown(&s), want, 0));

        let cpu_only = pixel_buffer(RGHA, false);
        fill(&cpu_only, 0, &texel);
        unsafe { s.present_cv_pixel_buffer(raw(&*cpu_only)) }.unwrap();
        let frame = s.lock_state().frame.clone();
        match frame {
            Some(Frame::Cpu { bgra, .. }) => assert!(bgra.chunks(4).all(|p| p == want)),
            _ => panic!("an RGhA buffer with no IOSurface is a CPU frame"),
        }
    }

    #[test]
    fn a_texture_of_the_other_format_is_refused() {
        let Some(device) = renderer() else { return };
        let half = surface("tests:metal-mismatch");
        half.set_format(SurfaceFormat::Rgba16Float);
        let bgra = filled(&device, [1, 2, 3, 255]);
        assert_eq!(
            unsafe { half.present_metal_texture(raw(&*bgra), None) },
            Err(SurfaceError::FormatMismatch {
                surface: SurfaceFormat::Rgba16Float,
                frame: SurfaceFormat::Bgra8
            })
        );
        let buffer = pixel_buffer(BGRA, true);
        assert!(matches!(
            unsafe { half.present_cv_pixel_buffer(raw(&*buffer)) },
            Err(SurfaceError::FormatMismatch { .. })
        ));
        assert_eq!(half.stats().presented, 0);
    }
}
