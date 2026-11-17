---
title: "Recipe: a libmpv player"
label: "Recipe: libmpv"
description: A video player whose picture libmpv renders into <sk-surface> and whose controls are a Vue or Svelte app. The render backends, which Skathi call each one uses, and wiring the controls.
section: Embedding
order: 6
---

A player is the textbook embedder: the native side owns decoding and rendering, the app owns everything a user touches. With libmpv the split looks like this:

<div class="pipeline">
<p class="pipeline-phase">Rust, your exe</p>

1. **mpv handle** `mpv_create`, `vo=libmpv`

   Playback, decoding, audio. Commands come from the app through `Builder::function`.

2. **Render context** `mpv_render_context_create`

   Renders each video frame into a target you give it: memory, an OpenGL framebuffer, or a Vulkan image.

3. **Present** `SurfaceHandle::present_*`

   The frame goes to the surface named `player`.

<p class="pipeline-phase" data-tone="run">The app</p>

4. **`<sk-surface name="player">`**

   The picture, sized and clipped by CSS, with `object-fit: contain`.

5. **Controls**

   Ordinary components over the picture: play, seek, volume, tracks. They call `mpv.*` functions and listen for `mpv.*` events.

</div>

## Pick a render backend

Upstream libmpv's render API has two backends, OpenGL and software. A libmpv build or fork whose render API renders through libplacebo on Vulkan (the `gpu-next` renderer) fits the Vulkan path.

| How mpv renders                           | mpv options and render API                                                                                                                           | Skathi call                                                                                       |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Software                                  | `vo=libmpv`, `hwdec=auto-copy`; `MPV_RENDER_API_TYPE_SW` with `MPV_RENDER_PARAM_SW_FORMAT` `"bgr0"`                                                  | `present_bgrx`, one CPU copy per frame                                                            |
| OpenGL through ANGLE on Skathi's device   | `vo=libmpv`, `hwdec=d3d11va`; `MPV_RENDER_API_TYPE_OPENGL` on an ANGLE display made from `d3d11_device()` (`EGL_ANGLE_device_d3d11`)                 | `present_d3d11_texture` with the texture behind mpv's FBO (`EGL_ANGLE_d3d_texture_client_buffer`) |
| OpenGL through ANGLE on its own device    | the same on a device of its own; the FBO's texture made `SHARED_NTHANDLE \| SHARED_KEYEDMUTEX`                                                       | `SharedTexture::with_keyed_mutex`, `present_shared`                                               |
| Vulkan through libplacebo (build or fork) | `vo=libmpv`, `gpu-api=vulkan`, `hwdec=d3d11va` or `vulkan`; the target image imported from a D3D12 shared resource, its semaphore from a D3D12 fence | `SharedTexture::with_fence`, `present_shared_fenced`                                              |

Start with software: it runs on every platform and needs no graphics interop. `bgr0` is exactly what `present_bgrx` takes, so the frame goes from mpv's buffer to the surface without conversion. Move to a GPU path when profiles show the CPU copy, typically at 4K or high frame rates. See [GPU frames on Windows](/embedding/gpu-frames/) for what each Skathi call needs.

## The render loop

Whatever the backend:

1. Register mpv's update callback (`mpv_render_context_set_update_callback`). It runs on mpv's thread; wake your render thread from it, and render nothing in the callback itself.
2. On the render thread, when `mpv_render_context_update` reports `MPV_RENDER_UPDATE_FRAME`, render a frame of `surface.info()`'s size and present it. Skip the render while `info().visible` is false: the element is hidden or scrolled away, and mpv keeps playing audio without spending the GPU.
3. With a fence, signal the frame's value after the render and wait for the release value before the next one renders into the same texture, or rotate two or three textures.
4. Resize the target when `surface.on_change` reports a new size, so the picture is rendered at the size it is shown.
5. On `SurfaceEvent::DeviceLost`, recreate the render target (and with ANGLE, the display) on the new device.

## Wiring the controls

The app does not need to know mpv exists. Give it a small, stable vocabulary:

```rust
let mpv = Arc::new(Mpv::new()?); // your wrapper over the mpv handle

let builder = skathi::Builder::new()
    .function("mpv.load", {
        let mpv = mpv.clone();
        move |call| {
            let (url,): (String,) = serde_json::from_str(call.args()).unwrap_or_default();
            match mpv.command(&["loadfile", &url]) {
                Ok(()) => call.resolve("null"),
                Err(e) => call.reject(e.to_string()),
            }
        }
    })
    .function("mpv.seek", {
        let mpv = mpv.clone();
        move |call| {
            let (seconds,): (f64,) = serde_json::from_str(call.args()).unwrap_or_default();
            mpv.command(&["seek", &seconds.to_string(), "absolute"]).ok();
            call.resolve("null");
        }
    })
    .function("mpv.pause", {
        let mpv = mpv.clone();
        move |call| {
            let (paused,): (bool,) = serde_json::from_str(call.args()).unwrap_or_default();
            mpv.set_property("pause", paused).ok();
            call.resolve("null");
        }
    });
let ui = builder.spawn()?;
```

mpv commands return quickly, so these answer on the Skathi thread. Anything that can wait on the network (opening a stream URL, probing) moves to a worker with `call.into_reply()`.

The other direction is mpv's property observer. Observe `time-pos`, `duration`, `pause` and `track-list` (`mpv_observe_property`), and forward each change from mpv's event thread with `Handle::emit`:

```rust
let events = ui.clone();
std::thread::spawn(move || {
    for change in mpv.property_changes() {
        let data = serde_json::json!({ "name": change.name, "value": change.value });
        if events.emit("mpv.property", &data.to_string()).is_err() {
            break;
        }
    }
});
```

`time-pos` changes many times a second. The app only paints once per frame, so forwarding every change is wasted work; send it at most every 100 ms, or once per frame from an `on_frame` hook (see [Native APIs](/embedding/native-api/#batching-per-frame)).

On the app side:

```vue
<script setup lang="ts">
import { onScopeDispose, ref } from "vue";
import { appHost } from "@skathi-native/runtime";

const position = ref(0);
const duration = ref(0);
const paused = ref(false);

const stop = appHost.on("mpv.property", (data) => {
  const { name, value } = data as { name: string; value: unknown };
  if (name === "time-pos") position.value = value as number;
  if (name === "duration") duration.value = value as number;
  if (name === "pause") paused.value = value as boolean;
});
onScopeDispose(stop);

const seek = (seconds: number) => appHost.call("mpv.seek", seconds);
const toggle = () => appHost.call("mpv.pause", !paused.value);
</script>

<template>
  <div class="player">
    <sk-surface name="player" class="picture" />
    <div class="controls">
      <button @click="toggle">{{ paused ? "Play" : "Pause" }}</button>
      <input
        type="range"
        min="0"
        :max="duration"
        :value="position"
        @change="seek(+($event.target as HTMLInputElement).value)"
      />
    </div>
  </div>
</template>
```

## Things that bite

- **Rendering at the wrong size.** Render at `surface.info()`'s width and height, which are physical pixels, not at the video's size: mpv scales better than a texture sampler, and the picture stays sharp on a high-DPI screen.
- **Rendering while hidden.** A minimised window or a closed panel still gets `MPV_RENDER_UPDATE_FRAME`. Check `info().visible` first.
- **Blocking mpv's thread.** The update callback and the event loop belong to mpv. Hand work to your own thread.
- **One surface, two players.** Each player presents to a surface of its own name. Two producers on one name fight over every frame.

Next: [Window, tray and hotkeys](/embedding/desktop/).
