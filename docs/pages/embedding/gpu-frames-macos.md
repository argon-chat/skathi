---
title: GPU frames on macOS
description: Present Metal textures, IOSurfaces and CVPixelBuffers to <sk-surface> without a CPU copy, from Skathi's own MTLDevice, another device or process, VideoToolbox or AVFoundation, synchronised with MTLSharedEvent.
section: Embedding
order: 6
since: "0.4"
---

On macOS, a frame for [`<sk-surface>`](/embedding/native-frames/) can stay on the GPU from the producer's render to the screen. Skathi renders with Metal; pick the path by where the producer's pixels are:

| Producer                                                           | Path                                                                                |
| ------------------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| Metal, can use Skathi's device                                     | `present_metal_texture` on `metal_device()`, with an `MTLSharedEvent`               |
| Metal on another device, another process, or anything on IOSurface | `SharedTexture::from_iosurface` or `from_mach_port`, optionally `with_shared_event` |
| VideoToolbox, AVFoundation, ScreenCaptureKit, Core Image           | `present_cv_pixel_buffer`                                                           |
| CPU, or anything else                                              | `present_bgra`, `present_bgrx`                                                      |

Every GPU path copies the frame on a command queue of the surface's own into one of up to four textures it owns. A wait on the producer's event happens on the GPU, and the window takes the frame once the copy has finished, so a slow producer never stalls the UI. Each surface has its own queue and release event: an event that never reaches its value holds back only the surface it was presented to. Textures are `BGRA8Unorm` or `BGRA8Unorm_sRGB` (the bytes are shown as they are), or `RGBA16Float` on a [half-float surface](#half-float-frames), 2D and single-sampled; alpha is straight.

The handles are raw pointers (`id<MTLDevice>`, `id<MTLTexture>`, `IOSurfaceRef`, `CVPixelBufferRef`), so any binding works: `objc2-metal`, the `metal` crate, or C and Swift on the other side of an FFI call.

## Metal on the renderer's device

`metal_device()` is the `MTLDevice` Skathi composites with, `NoDevice` until a window has drawn. Render on a command queue of your own into a texture made on it, signal an `MTLSharedEvent` of yours, and present the texture with that event and value. Nothing blocks: the surface's queue waits for your value on the GPU, copies, and signals the surface's release event. `present_metal_texture` returns a `FramePresent`: the frame's id ([when it reached the screen](/embedding/native-frames/#knowing-when-a-frame-is-on-screen)) and that release value. Render into the texture again once `release_event()` reaches it (`wait` on the CPU, or `encodeWaitForEvent` on your queue), or keep a few textures in rotation.

This producer, with `objc2` and `objc2-metal` 0.3, clears a texture to a moving colour at the element's size:

```rust
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLLoadAction,
    MTLPixelFormat, MTLRenderPassDescriptor, MTLSharedEvent, MTLStorageMode, MTLStoreAction,
    MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use skathi::{SurfaceError, SurfaceHandle};

fn main() {
    let builder = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .setup(|host| {
            let surface = host.surface("preview");
            std::thread::spawn(move || produce(surface));
            Ok(())
        });
    if let Err(e) = builder.run_on_main_thread(|exit| i32::from(!exit.clean)) {
        eprintln!("{e}");
        std::process::exit(2);
    }
}

struct Gpu {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    size: (u32, u32),
}

fn produce(surface: SurfaceHandle) {
    let start = Instant::now();
    let mut gpu: Option<Gpu> = None;
    let mut frame = 0;
    loop {
        std::thread::sleep(Duration::from_millis(16));
        let info = surface.info();
        if !info.visible || info.width == 0 {
            continue;
        }
        let size = (info.width, info.height);
        if gpu.as_ref().is_none_or(|g| g.size != size) {
            // `NoDevice` until the window has drawn once.
            let Ok(device) = surface.metal_device() else { continue };
            let device = unsafe { Retained::retain(device.as_raw().cast::<ProtocolObject<dyn MTLDevice>>()) }.unwrap();
            gpu = Some(Gpu {
                queue: device.newCommandQueue().unwrap(),
                event: device.newSharedEvent().unwrap(),
                texture: render_target(&device, size),
                size,
            });
            frame = 0;
        }
        let g = gpu.as_ref().unwrap();
        let t = start.elapsed().as_secs_f64();
        frame += 1;
        clear(g, [0.5 + 0.5 * t.sin(), 0.5 + 0.5 * (t * 1.3).sin(), 0.6], frame);
        let texture = Retained::as_ptr(&g.texture).cast_mut().cast();
        let event = Retained::as_ptr(&g.event).cast_mut().cast();
        match unsafe { surface.present_metal_texture(texture, Some((event, frame))) } {
            Ok(presented) => {
                // The texture is the surface's to read until its release event reaches this value.
                let released = presented.release;
                surface.release_event().map(|e| e.wait(released, Duration::from_millis(100))).ok();
            }
            Err(SurfaceError::DeviceChanged | SurfaceError::WrongDevice) => gpu = None,
            Err(_) => {}
        }
    }
}

fn render_target(device: &ProtocolObject<dyn MTLDevice>, (w, h): (u32, u32)) -> Retained<ProtocolObject<dyn MTLTexture>> {
    let desc = MTLTextureDescriptor::new();
    desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    unsafe {
        desc.setWidth(w as usize);
        desc.setHeight(h as usize);
    }
    desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    desc.setStorageMode(MTLStorageMode::Private);
    device.newTextureWithDescriptor(&desc).unwrap()
}

fn clear(g: &Gpu, [red, green, blue]: [f64; 3], frame: u64) {
    let command = g.queue.commandBuffer().unwrap();
    let pass = MTLRenderPassDescriptor::new();
    let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
    color.setTexture(Some(&g.texture));
    color.setLoadAction(MTLLoadAction::Clear);
    color.setStoreAction(MTLStoreAction::Store);
    color.setClearColor(MTLClearColor { red, green, blue, alpha: 1.0 });
    command.renderCommandEncoderWithDescriptor(&pass).unwrap().endEncoding();
    command.encodeSignalEvent_value(ProtocolObject::from_ref(&*g.event), frame);
    command.commit();
}
```

`crates/sk-app/examples/surface.rs` in the Skathi repository is a complete program with every path.

`metal_device().queue()` is the renderer's own queue: work committed there delays the next frame, so a producer keeps its own. A texture made on another `MTLDevice` returns `SurfaceError::WrongDevice`; share it as an IOSurface instead. If the surface's textures are all still in flight (the producer outruns the window, or its event is never signalled), a present to it returns `SurfaceError::Busy` and the frame can be dropped. Other surfaces keep showing their frames.

## IOSurfaces from another device or process

An IOSurface is the macOS way to share GPU memory between devices, APIs and processes. Create it with pixel format `'BGRA'` (`kCVPixelFormatType_32BGRA`), render into it (`newTextureWithDescriptor:iosurface:plane:` on any `MTLDevice`, `CGLTexImageIOSurface2D` in OpenGL), and open it on Skathi's device:

- **In this process:** `SharedTexture::from_iosurface(surface)` with the `IOSurfaceRef`. It is retained while the `SharedTexture` lives.
- **From another process:** the producer makes a mach port with `IOSurfaceCreateMachPort` and sends it (XPC with `xpc_dictionary_set_mach_send`, or a mach message); with the send right in this process, `SharedTexture::from_mach_port(port)` looks the surface up. The port stays yours to deallocate.

Synchronise with an `MTLSharedEvent`, the Metal counterpart of a shared fence: `with_shared_event(event)` for one in this process (made on any device), `with_shared_event_handle(handle)` for the `MTLSharedEventHandle` another process made with `newSharedEventHandle` and sent over XPC. The producer picks the values per frame: the copy waits on the GPU until the event reaches `wait`, then signals it to `release`, after which the producer may write the surface again.

```rust
// Once per surface; again after SurfaceError::DeviceChanged.
let shared = unsafe { skathi::SharedTexture::from_mach_port(port)?.with_shared_event_handle(handle)? };
// Per frame, after the producer signalled 2n + 1:
surface.present_shared_fenced(&shared, 2 * n + 1, 2 * n + 2)?;
// The producer waits for 2n + 2 (encodeWaitForEvent or notifyListener) before rendering into it again.
```

Without an event, `present_shared(&shared)` copies whatever the IOSurface holds when the copy runs: the producer must have finished its writes (`waitUntilCompleted`, `glFinish`), and rotates a few IOSurfaces so it never writes one still being copied. Keyed mutexes and NT handles are Direct3D objects and return `SurfaceError::NotSupported` here.

A video player that renders through OpenGL, such as libmpv's render API, fits this path: bind an IOSurface to the texture of its framebuffer with `CGLTexImageIOSurface2D`, render, `glFinish`, and present it.

## CVPixelBuffers

`present_cv_pixel_buffer(buffer)` takes a `CVPixelBufferRef` as VideoToolbox decodes it and as `AVPlayerItemVideoOutput`, `AVCaptureVideoDataOutput` and ScreenCaptureKit deliver it:

| Pixel format                             | Shown                                                                            |
| ---------------------------------------- | -------------------------------------------------------------------------------- |
| `kCVPixelFormatType_32BGRA`              | copied on the GPU (on the CPU when it has no IOSurface)                          |
| `420v`, `420f` (bi-planar 4:2:0, 8-bit)  | converted to BGRA on the GPU, video or full range                                |
| `kCVPixelFormatType_64RGBAHalf` (`RGhA`) | on a half-float surface: copied on the GPU (on the CPU when it has no IOSurface) |

The conversion uses the buffer's YCbCr matrix attachment (BT.601, BT.709 or BT.2020; BT.709 when it names none). The buffer is retained until the GPU has read it, so release yours right after the call. It must be fully written; a decoder's output is. Ask a decoder or a capture output for one of these formats (`kCVPixelBufferPixelFormatTypeKey`), with IOSurface backing (`kCVPixelBufferIOSurfacePropertiesKey`), which VideoToolbox and AVFoundation give by default.

```rust
// In AVPlayerItemVideoOutput's or a VTDecompressionSession's callback:
unsafe { surface.present_cv_pixel_buffer(pixel_buffer.cast())? };
```

## Half-float frames

A surface set to `SurfaceFormat::Rgba16Float` takes `RGBA16Float` Metal textures, IOSurfaces with pixel format `'RGhA'`, and `'RGhA'` CVPixelBuffers (`kCVPixelFormatType_64RGBAHalf`), on the same paths and with the same events as above. A `BGRA8Unorm` texture, a `'BGRA'` IOSurface or a 4:2:0 buffer on it returns `SurfaceError::FormatMismatch`, and so does a half-float one on an 8-bit surface.

```rust
surface.set_format(skathi::SurfaceFormat::Rgba16Float);
// … render into an RGBA16Float texture on metal_device(), signal `frame`, then:
let presented = unsafe { surface.present_metal_texture(texture, Some((event, frame)))? };
```

The values are linear extended sRGB with 1.0 as SDR white: an `'RGhA'` buffer in `kCGColorSpaceExtendedLinearSRGB`, which is what AVFoundation and Core Image hand out when asked for linear half floats. A buffer's colour-space attachments are not read. The frame stays half float in the surface's textures. A window on a screen with EDR headroom draws HDR while it shows a visible half-float surface whose [`dynamic-range-limit`](/css/dynamic-range-limit/) allows more than 1.0 (see [HDR windows](/embedding/native-frames/#hdr-windows)): the element then shows the frame's own values, up to that limit and the screen's current headroom. On macOS 26 and later the window tags its layer with the headroom that limit allows, so the system raises the screen's headroom only that far; earlier versions may raise it to the screen's maximum. In a standard window, when an element draws the frame, a compute pass on the surface's queue maps it into an 8-bit texture (clamped to 0…1, then the exact sRGB curve; see [half-float frames](/embedding/native-frames/#half-float-frames)), and the window's copy waits for that pass on the GPU; values above 1.0 are clipped there.

## Choosing, in short

- **One process, your own Metal renderer?** Render on `metal_device()` with your own queue and event, present with `present_metal_texture`, rotate two or three textures.
- **Decoded video or screen capture?** `present_cv_pixel_buffer`, frame by frame.
- **Another process, or a device or API you cannot change?** Share an IOSurface over a mach port, with an `MTLSharedEvent` through its handle.
- **Not sure yet?** Start with `present_bgra`. It works everywhere, and moving to a GPU path later changes only the producer.

Next: [Window, tray and hotkeys](/embedding/desktop/).
