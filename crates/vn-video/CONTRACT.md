# `vn-video` — the `<video>` contract

What this crate promises, what the element layer owes it, and where it
deliberately differs from a browser. `crates/vn-tree/src/video.rs` is the
implementation of the host half; `research/reports/m11/video.md` is the research
that chose the design and `research/reports/m12/video.md` is what was actually
built and measured.

---

## 1. The thesis, in one paragraph

We ship no codec. Windows already contains a complete, hardware-accelerated,
correctly-licensed `HTMLMediaElement` implementation called `IMFMediaEngine`,
and macOS contains one called `AVPlayer`. Both own the demuxer, the decoder, the
audio renderer and the A/V master clock; both expose
play/pause/seek/rate/volume/duration/readyState plus a pull-the-current-frame
call. So this crate is a thin portable element over a `VideoBackend` trait
pitched at exactly that altitude, and the platform-specific code is a rename.

The alternative — bundling ffmpeg — costs 3–70 MB of binary, 60–160 MiB of RAM
per stream, a per-frame CPU colour conversion, an 8.3 MB/frame PCIe upload and a
patent-licensing question, in exchange for codecs Windows already decodes at
27–31× realtime without the frame leaving VRAM. The numbers are in the m11
report §3 and §8.

---

## 2. `VideoBackend`, and why it is shaped like that

| trait member | Windows | macOS (designed, §8) |
| --- | --- | --- |
| `load` | `IMFMediaEngine::SetSource(BSTR url)` | `AVPlayerItem(asset: AVURLAsset)` |
| `play` / `pause` | `Play` / `Pause` | `AVPlayer.play()` / `.pause()` |
| `seek` | `SetCurrentTime` | `AVPlayer.seek(to:)` |
| `set_volume` / `set_muted` | `SetVolume` / `SetMuted` | `AVPlayer.volume` / `.isMuted` |
| `set_rate` | `SetPlaybackRate` | `AVPlayer.rate` |
| `set_looping` | `SetLoop` | `AVPlayerLooper` |
| `current_time` / `duration` | `GetCurrentTime` / `GetDuration` | `currentTime()` / `duration` |
| `ready_state` / `network_state` | `GetReadyState` / `GetNetworkState` (same 0–4 / 0–3 enums) | `AVPlayerItem.status` + `isPlaybackLikelyToKeepUp` |
| `buffered` / `seekable` | `GetBuffered` / `GetSeekable` | `loadedTimeRanges` / `seekableTimeRanges` |
| `native_size` | `GetNativeVideoSize` | `AVAssetTrack.naturalSize` |
| `can_play_type` | `CanPlayType` (same `""`/`maybe`/`probably`) | `AVURLAsset.isPlayableExtendedMIMEType` |
| `present` | `OnVideoStreamTick` + `TransferVideoFrame` | `hasNewPixelBuffer` + `copyPixelBuffer` |
| `drain_events` | `IMFMediaEngineNotify` queue | KVO + `AVPlayerItemDidPlayToEndTime` + periodic observer |
| `suspend` | `SetSource("")` — tears the pipeline down | `replaceCurrentItem(with: nil)` |

**No platform type appears in `backend.rs`, `state.rs` or `element.rs`.** That is
the invariant that keeps the port honest; if a future backend needs the trait to
grow, it grows in these terms, not in MF's.

### Rules a backend must keep

1. **It owns the clock and the audio.** It is never ticked with a wall-clock
   delta and is never asked to render audio. Both platforms already do this,
   correctly, in the OS.
2. **Getters are cheap and may be called per frame.**
3. **Setters are idempotent.** The element layer pushes every prop every frame;
   a setter that re-fires an event on an unchanged value produces a
   `volumechange` storm at the frame rate. (`state.rs` also dedupes, so this is
   belt and braces — but a backend that fires events from a no-op setter would
   defeat it for anything the state machine does not cache.)
4. **`present` is UI-thread only.** It touches the graphics device.
5. **`drain_events` never re-enters the tree.** Platform callbacks arrive on
   platform threads and must be *queued*. Collect first, dispatch after — the
   same discipline `lottie::tick` documents, for the same reason: a JS handler
   can mutate the tree.

---

## 3. What the element layer owes the crate

Per frame, from the host's pump, in this order:

```rust
// once per <video> node, in a side map keyed by node id
let mut state = VideoState::new(vn_video::backend_for_this_platform()?);
state.set_source(MediaSource::Path(path))?;

// every frame, BEFORE the draw
let advance = state.advance(Instant::now(), &FrameRequest { max_size });
for event in advance.events { /* dispatch to JS, gated on the listener bit */ }

// every frame, in the element build
let element = vn_video::video(&mut state, &options, window, cx);
```

* **`take_retired()` must be drained every frame** (`vn_video::video` does it).
  gpui keys atlas tiles by `RenderImage::id` and every decoded frame mints a
  fresh id, so a 60 fps video leaks one atlas tile per frame otherwise. This is
  the identical hazard `crates/vn-lottie/CONTRACT.md` §3 documents.
* **`take_all_images()` on teardown**, through a pending-drops queue if the GC
  site has no `Window` — which it does not.
* **`suspend()` when the node stops being painted.** See §5.

---

## 4. The event set

Every event below is an `MF_MEDIA_ENGINE_EVENT` under 1000, a range Microsoft
documents as "events defined in HTML 5 for media elements". So the OS raises
them with HTML semantics and we rename.

| HTML event | MF code | `EV_*` | v1 |
| --- | --- | --- | --- |
| `loadstart` | 1 | `EV_LOADSTART = 17` | yes |
| `error` | 5 | `EV_ERROR = 33` | yes |
| `emptied` | 6 | `EV_EMPTIED = 34` | yes |
| `stalled` | 7 | folded into `waiting` | — |
| `play` / `pause` | 8 / 9 | `EV_PLAY = 22` / `EV_PAUSE = 24` | yes |
| `loadedmetadata` / `loadeddata` | 10 / 11 | `EV_LOADEDMETADATA = 18` / `EV_LOADEDDATA = 19` | yes |
| `waiting` / `playing` | 12 / 13 | `EV_WAITING = 31` / `EV_PLAYING = 23` | yes |
| `canplay` / `canplaythrough` | 14 / 15 | `EV_CANPLAY = 20` / `EV_CANPLAYTHROUGH = 21` | yes |
| `seeking` / `seeked` | 16 / 17 | `EV_SEEKING = 25` / `EV_SEEKED = 26` | yes |
| `timeupdate` | 18 | `EV_TIMEUPDATE = 27` | yes, **re-cadenced** |
| `ended` | 19 | `EV_ENDED = 32` | yes |
| `ratechange` / `durationchange` / `volumechange` | 20 / 21 / 22 | `EV_RATECHANGE = 29` / `EV_DURATIONCHANGE = 28` / `EV_VOLUMECHANGE = 30` | yes |
| `progress` / `suspend` / `abort` | 2 / 3 / 4 | — | tier 2 |

**`timeupdate` is ours, not MF's.** The spec says "about 4 Hz to 66 Hz, and no
more often than every 250 ms". MF raises it on the decoder's cadence, which on
this machine is 165 Hz — a JS dispatch on every frame, for a readout that
changes by 6 ms. `VideoState::advance` drops MF's and emits one on a 250 ms
clock, plus one at every boundary the spec calls out (`seeked`, `ended`,
`loadeddata`) so a bound UI is never left stale. The stream is then identical on
every platform and matches a browser's.

**Payload** (`EventPayload`): `a` = `currentTime`, `b` = `duration`, `c` =
`readyState`, `d` = `playbackRate`. For `error`: `a` = `MediaError.code` (1–4),
`b` = `currentTime`, `key` = the advisory text.

`timeupdate` is gated on the listener bit before the payload is built, like
`<lottie>`'s `enterFrame`.

---

## 5. Presentation — what v1 does and what replaces it

`TransferVideoFrame` puts the current frame in a texture we own, **on the
renderer's device**, already `DXGI_FORMAT_B8G8R8A8_UNORM` (gpui's exact atlas
format) and already cropped to the display aperture. From there it is one
`CopySubresourceRegion` — 0.11–0.18 ms of pure VRAM traffic, measured — from
being a gpui atlas tile.

**v1 does not take that copy.** Doing so needs a new method on `DirectXAtlas` or
the empty `PrimitiveBatch::Surfaces` / `draw_surfaces()` stub filled in, i.e.
changes to `directx_atlas.rs` / `directx_renderer.rs` in the fork, and this round
was scoped to `directx_devices.rs`. So v1 goes GPU → system memory →
`RenderImage` → atlas, the one currency the renderer already accepts from
outside.

Two things make that affordable, and both are load-bearing:

* **Frames are produced at the size they will be PAINTED.** MF's video processor
  is scaling anyway; asking it for the element's box instead of the file's coded
  size turns 8.3 MB/frame into 1.4 MB/frame for a 1080p clip in an 800×450 box.
  `FrameRequest::max_size` carries the box; `fit_within` clamps to the video's
  aspect ratio and never upscales.
* **One texture pair per size, reused forever.** No VRAM churn while playing.

`object-fit` is then applied by gpui's own `ObjectFit` on the image element, so
`contain`/`cover`/`fill`/`scale-down`/`none` behave exactly as they do on `<img>`
— the same code path, not a second implementation. The **default is `contain`**,
which is the HTML spec's rule for a video (not the CSS initial value `fill`).

**The replacement, when the atlas work lands:** keep `TransferVideoFrame` as it
is, drop `Presenter::transfer`'s staging texture and `Map`, and hand the target
texture to a `blit_into_tile(key, src_texture, src_box)` on `DirectXAtlas`. The
`VideoFrame` struct becomes a texture handle instead of an `Arc<RenderImage>`;
nothing above `backend.rs` changes. On macOS the same seam is already served by
gpui's existing `PaintSurface` / `CVPixelBuffer` path.

### Lifetime

* A `<video>` node that stops being built — detached, `display: none`, parked in
  a `<KeepAlive>` cache container — is **suspended**: `pause()` plus
  `SetSource("")`, which releases the decoder, the DPB and the video-processor
  pool. That is ~28 MiB of RAM and ~68 MiB of VRAM per 1080p stream, so unlike
  `<lottie>` (whose parked clock costs arithmetic) it is not something to leave
  running. A browser does not keep decoding a detached video either.
* Retired `RenderImage`s go back through `Window::drop_image`. See §3.

---

## 6. The JS surface

`HTMLMediaElement` is the one element whose interesting state lives in the HOST:
`currentTime` moves inside the OS pipeline sixty times a second and no prop write
puts it there. So the surface splits, the same way `getBoundingClientRect` and
`getComputedStyle` already do:

| host function | answers |
| --- | --- |
| `__vn_media_state(id)` | `name=value` lines: `currentTime`, `duration`, `paused`, `ended`, `seeking`, `readyState`, `networkState`, `volume`, `muted`, `playbackRate`, `loop`, `videoWidth`, `videoHeight`, `buffered`, `seekable`, `error`, `errorMessage` |
| `__vn_media_call(id, verb, arg)` | 0 `play` · 1 `pause` · 2 `currentTime = arg` · 3 `volume = arg` · 4 `muted = arg` · 5 `playbackRate = arg` · 6 `loop = arg` |
| `__vn_media_can_play(id, type)` | `""` / `"maybe"` / `"probably"`, probed live |
| `__vn_media_events(id)` | drains the node's recent event names — a bridge and a test surface, not the event channel |

**What is still missing, and it is one file.** The JS runtime
(`packages/vue-native/src/runtime.js`) is owned by another track this round and
was deliberately not touched, so it does not yet know the media event codes or
the media properties. Until it does, `@timeupdate` in a template warns
("this event type is not delivered by the native host yet") and the events, which
the host dispatches correctly on the `EV_*` channel, land nowhere.

The change is additive and lives in four places:

1. `EV_CODES` — add the eighteen names from §4 with their codes.
2. `NO_BUBBLE` — add all eighteen. Media events fire at the target only.
3. `synthesize` — a `mediaDetail(a, b, c, d)` case, so `e.currentTime` and
   `e.detail.currentTime` both read; `error` gets `e.target.error = { code: a,
   message: key }`.
4. `ElementProto` — `play()`/`pause()`/`load()`/`canPlayType()` and the
   `currentTime`/`duration`/`paused`/`ended`/`volume`/`muted`/`playbackRate`/
   `videoWidth`/`videoHeight`/`readyState`/`buffered`/`seekable`/`error`
   accessors, over the four host functions above.

Nothing in Rust changes when it lands. `apps/hello`'s MediaPage carries a small
`useMediaElement` composable that does (4) in userland today and is deleted the
day (1)–(4) exist.

---

## 7. Advisories — the compat-first list

Following `memory/compat-first-policy.md`: never diverge silently.

| feature | v1 behaviour | could become |
| --- | --- | --- |
| `controls` | advisory; build the transport in Vue | v2, as a userland component, not native chrome |
| `preload` | accepted, ignored, advisory | the OS pipeline picks its own read-ahead |
| `crossorigin`, `disablepictureinpicture`, `disableremoteplayback`, `autopictureinpicture` | advisory | — |
| `playsinline` | accepted in silence — we are never fullscreen by default | — |
| `srcObject` / `MediaStream` | advisory | v3 at the earliest; this build exposes no `getUserMedia` |
| `data:` URIs in `src` | advisory | `IMFMediaEngineEx::SetSourceFromByteStream` + `MFCreateMFByteStreamOnStream`; deliberately not v1, because the point of the path-based load is that the OS owns the read-ahead buffer and a 30-minute clip never becomes a `Vec<u8>` |
| `http(s)://` in `src` | advisory, same as `<img>` today | MF's network source can do it; keep the two elements consistent |
| `<source>` / `<track>` children | inert (they render nothing, as in a browser) | `<track>` is v2 — MF's MKV source does expose subtitle streams |
| WebCodecs, MSE, EME | not implemented | WebCodecs is the plausible one (`IMFTransform` ≈ `VideoDecoder`); EME is explicitly not planned |
| `requestFullscreen`, Picture-in-Picture | not implemented | a windowless-swapchain escape hatch exists but takes the video out of the scene graph |

---

## 8. Codecs, and the owner constraint

**Guaranteed baseline = what Windows ships inbox: H.264 + AAC in MP4.**
Everything else — HEVC, VP9, AV1, VP8/Opus/Vorbis — arrives as a separately
installable Store package that an enterprise image may have stripped and that an
OEM may or may not have provisioned.

So:

* `can_play_type` is a **live probe of this machine**, never a table.
* A failed load produces an advisory that *names the missing decoder and the
  fix* — built by probing HEVC/AV1/VP9 at the moment of failure and reporting
  which of them this machine actually lacks, plus the `ffmpeg` one-liner that
  re-encodes to the baseline. `MediaError::advisory` carries it; the element
  layer prints it once whether or not an `@error` handler exists, because the
  developer without a handler is exactly the one who needs to be told.
* macOS will have a *different* set (HEVC/H.264/ProRes free, VP9 and AV1 only on
  recent silicon), which is the second reason the probe cannot be a table.

### `av1-software` (designed, not built)

An opt-in cargo feature for a vendored **dav1d** (BSD-2), for machines with no
`Microsoft.AV1VideoExtension`. It is off by default and always will be:

* dav1d's own documented 1080p footprint is **42.8–162.9 MB of RAM**, against
  ~28 MiB for the whole OS pipeline;
* it adds roughly 1 MB of binary;
* software decode outputs to system memory, so every frame costs a CPU colour
  conversion *and* an upload the OS path does not pay;
* AV1 is royalty-free by AOM design, so there is no licensing reason not to —
  only a memory one, and the owner's memory budget is the constraint.

If it is ever built: bound `frame_threads`/`tile_threads` to 2 and set
`max_frame_delay = 1`, which is what keeps dav1d at the bottom of that RAM range;
expose it as a *fallback* the MF backend reaches for only after `CanPlayType`
answers `""` for AV1; and document the resident cost in the app's own advisory,
not just in this file.

---

## 9. macOS readiness

The port is a new `backend/avf.rs` and nothing else. Concretely:

* `AVURLAsset` + `AVPlayerItem` + `AVPlayer` for the transport; `AVPlayerLooper`
  for `loop`.
* `AVPlayerItemVideoOutput` with
  `kCVPixelBufferMetalCompatibilityKey: true` and
  `kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA` — the same
  "ask for BGRA and let the OS's video processor do the conversion" trick as
  `MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT`.
* `hasNewPixelBufferForItemTime` → `present`'s "is there a new frame";
  `copyPixelBuffer(forItemTime:)` → the frame.
* Events: KVO on `AVPlayerItem.status` (`loadedmetadata`, `error`),
  `isPlaybackLikelyToKeepUp` (`canplay`/`waiting`), `AVPlayer.timeControlStatus`
  (`play`/`playing`/`pause`), `AVPlayerItemDidPlayToEndTimeNotification`
  (`ended`), `seek(to:completionHandler:)` (`seeking`/`seeked`). `timeupdate`
  needs no periodic observer at all — `VideoState` synthesises it.
* Presentation is *already* better there than here: gpui's `PaintSurface`
  carries a `CVPixelBuffer` under `#[cfg(target_os = "macos")]` and
  `Window::paint_surface` exists. So macOS can take the zero-copy path
  immediately, and it is Windows that has the stub to fill in.

The one thing the trait may need to grow for it: an audio-session category
(`AVAudioSession` is iOS-only, so probably not on macOS). Nothing in §2 has to
be reshaped.
