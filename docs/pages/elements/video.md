---
title: "<video>"
label: "<video>"
metaTitle: "<video>: play video through the OS player"
description: How <video> plays through Media Foundation, AVFoundation or GStreamer, which attributes, properties and events it supports, how frames reach the screen, sizing and CSS, and why <audio> plays nothing.
section: Elements
order: 6
---

`<video>` plays a video file or stream with the operating system's player: Media Foundation on Windows, AVFoundation on macOS, GStreamer on Linux. Skathi ships no codecs. The OS player reads the file, decodes it, plays its sound and keeps audio and video in sync, and Skathi draws each frame into the element's box like any other picture, so CSS clips, rounds, fades and layers it.

There is no built-in transport bar. Build controls in Vue or Svelte; the [web-platform reference](/web-platform/video/) has a custom-controls example.

## A minimal example

```vue
<script setup lang="ts">
import { ref } from "vue";

const video = ref<HTMLVideoElement | null>(null);
const playing = ref(false);

function toggle() {
  if (video.value!.paused) video.value!.play();
  else video.value!.pause();
}
</script>

<template>
  <video
    ref="video"
    class="clip"
    src="assets/intro.mp4"
    poster="assets/intro.png"
    muted
    @playing="playing = true"
    @pause="playing = false"
  />
  <button @click="toggle">{{ playing ? "Pause" : "Play" }}</button>
</template>

<style scoped>
.clip {
  width: 100%;
  aspect-ratio: 16 / 9;
  border-radius: 8px;
}
</style>
```

```svelte
<script lang="ts">
  let video: HTMLVideoElement;
  let playing = $state(false);
</script>

<video
  bind:this={video}
  class="clip"
  src="assets/intro.mp4"
  muted
  onplaying={() => (playing = true)}
  onpause={() => (playing = false)}
></video>
<button onclick={() => (video.paused ? video.play() : video.pause())}>{playing ? "Pause" : "Play"}</button>
```

## Attributes

| Attribute         | What it does                                                                                                  |
| ----------------- | ------------------------------------------------------------------------------------------------------------- |
| `src`             | The video to play; see [Sources](#sources). A changed value loads a new video. An empty one loads nothing.    |
| `poster`          | An image shown until the first frame, and after a failed load. It follows `object-fit` and the corner radius. |
| `autoplay`        | Plays when loaded. It stays live: turning it off pauses.                                                      |
| `loop`, `muted`   | Booleans, live: changing the attribute changes playback.                                                      |
| `volume`          | Skathi only: 0 to 1, clamped.                                                                                 |
| `playbackrate`    | Skathi only: the rate. Also spelled `playback-rate`. A value of 0 or below is ignored.                        |
| `currenttime`     | Skathi only: each write seeks there, even to the same value. Also spelled `current-time`.                     |
| `width`, `height` | Size hints in CSS pixels, unitless. CSS wins.                                                                 |
| `playsinline`     | Accepted and ignored.                                                                                         |

A boolean attribute counts as on when present, unless its value is `"false"`. The live `autoplay`, `muted` and `loop`, and the extra `volume`, `playbackrate` and `currenttime`, let a template drive playback from state without a ref.

These print an advisory and do nothing: `controls` (no controls are drawn), `crossorigin`, `preload` (the OS decides how far to read ahead), `disablepictureinpicture`, `disableremoteplayback`, `autopictureinpicture` and `srcobject` as an attribute. `<source>` and `<track>` children are ignored: only the `src` attribute is read.

## Properties and methods

`currentTime`, `duration`, `paused`, `ended`, `seeking`, `readyState`, `networkState`, `volume`, `muted`, `playbackRate`, `loop`, `videoWidth` and `videoHeight` read the player's state. `volume`, `muted`, `playbackRate` and `loop` report the last value set, not one read back from the OS. Before a video is loaded they read the HTML defaults: `duration` is `NaN`, `paused` is `true`.

- `play()` returns a promise that resolves at once and never rejects. Wait for `playing` to know playback started. There is no autoplay policy: a video plays without a user gesture, sound or not.
- `pause()` pauses.
- `load()` does nothing and prints an advisory. To reload, clear `src` and set it again.
- `canPlayType()` asks the system's decoders, so the answer differs between machines.
- `error` is a plain `{ code, message }` object. The codes are the HTML ones; the message says what is missing, such as the decoder this machine lacks.
- `buffered` and `seekable` are plain objects with `length`, `start(i)` and `end(i)`.
- `requestFullscreen()` works as on any element.

Settings made before the video loads, such as `muted`, `volume`, `playbackRate`, `loop`, `play()` and `pause()`, are kept and applied when it loads. A `currentTime` set before then is dropped; set it after `loadedmetadata`.

Not available: `requestVideoFrameCallback`, `requestPictureInPicture`, `captureStream`, `setSinkId`, `fastSeek`, `textTracks`, `defaultMuted`, `defaultPlaybackRate` and the `poster`, `preload` and `crossOrigin` properties. `src` reads the attribute as written, not a resolved URL. Drawing a video on a canvas with `drawImage` draws nothing yet.

## Events

`loadstart`, `loadedmetadata`, `loadeddata`, `canplay`, `canplaythrough`, `play`, `playing`, `pause`, `seeking`, `seeked`, `timeupdate`, `durationchange`, `ratechange`, `volumechange`, `waiting`, `ended`, `error` and `emptied` fire. `progress`, `suspend`, `abort`, `stalled` and `resize` do not.

Every media event carries `currentTime`, `duration`, `readyState` and `playbackRate` on the event and on `detail`, typed as `SkMediaEvent`, so a listener needs no read-back. Media events do not bubble. `timeupdate` fires every 250 ms while the position moves, and once at `seeked`, `ended` and `loadeddata`. Events are collected from the player and dispatched once per frame.

## Sources

| `src`                            | Plays                                                                                             |
| -------------------------------- | ------------------------------------------------------------------------------------------------- |
| A relative or root-relative path | The app's file. In a packaged app it is read from the package, in process.                        |
| `https:`, `http:`                | Streamed by the OS player. macOS refuses cleartext `http:`.                                       |
| `blob:`                          | A picked file plays from its path; an in-memory Blob plays from memory.                           |
| `data:`                          | Plays from memory on macOS and Linux. On Windows the bytes are written to a temporary file first. |
| `file:`, an absolute path        | The file on disk.                                                                                 |

There is no Media Source Extensions API and no Encrypted Media Extensions. Whether a playlist URL such as HLS plays is up to the OS player.

## How it is drawn

The OS player decodes on its own threads. Once per frame, on the UI thread, Skathi takes the newest decoded frame at the size it is shown and draws it into the element's box. Frames are decoded at the painted size and never upscaled.

- **Windows.** Media Foundation decodes on Skathi's own Direct3D 11 device and the frame is copied into the renderer's texture atlas on the GPU, so it never comes back to the CPU. If the atlas refuses the copy, Skathi falls back to a CPU readback once, with a warning.
- **macOS.** VideoToolbox decodes and scales on the GPU; each frame is then copied into a texture on the CPU.
- **Linux.** GStreamer's `playbin3` decodes into BGRA at the painted size, uploaded each frame. GStreamer is loaded at run time, so the app starts on a machine without it; a missing plugin produces an error that names the package to install.

Frames are 8-bit BGRA: video is shown in SDR, and [`dynamic-range-limit`](/css/dynamic-range-limit/) has no effect on it. After a GPU device loss, every player is rebuilt and resumes at the same position.

A video keeps playing while it is hidden, scrolled out of view or in a minimized window. Pause it yourself if you want it to stop; `appWindow` fires [`minimize`](/window-api/event-minimize/) when the window is minimized. A video removed from the document, including one kept alive by `<KeepAlive>`, fires `timeupdate` and `pause` and stays paused when it comes back.

A `<video>` in a [pop-out window](/elements/sk-window/) plays there: its frames are uploaded in the window that paints it.

## Size and CSS

The element has no default size. Until the video's dimensions are known it is sized by CSS and the `width` and `height` attributes alone; once they are known, they fill whichever side CSS left unset, keeping the video's ratio or a CSS [`aspect-ratio`](/css/aspect-ratio/). Give a video a width and an `aspect-ratio` so the layout does not shift when it loads.

- [`object-fit`](/css/object-fit/) defaults to `contain`, as in browsers; `object-position` applies.
- [`border-radius`](/css/border-radius/) and [`corner-shape`](/css/corner-shape/) clip the picture.
- `transform`, `opacity` and `z-index` apply, and elements can sit over the video: it is drawn in the page, not in a separate native layer.

## Codecs and platforms

| Platform | Player                                       | `canPlayType` asks                                                        |
| -------- | -------------------------------------------- | ------------------------------------------------------------------------- |
| Windows  | Media Foundation                             | the installed decoders, for H.264, HEVC, AV1, VP8, VP9, MPEG-2 and Theora |
| macOS    | AVFoundation (`AVPlayer`)                    | `AVURLAsset.isPlayableExtendedMIMEType`                                   |
| Linux    | GStreamer `playbin3` (`playbin` before 1.22) | the GStreamer registry for a demuxer, parser and decoder                  |

On Windows only H.264 is present on every install; HEVC, AV1 and VP9 come from Store extensions, and the error for a missing decoder names the extension. Keep an H.264 fallback. Sound goes to the system's default output device.

`<video>` needs the host's `video` Cargo feature, on by default. Without it the element is an empty box, `canPlayType()` returns `""` and the first one prints a warning; see [Features and size](/embedding/features/).

## Audio

`<audio>` plays nothing. It has the media element's JavaScript API, but no player behind it: `play()` resolves, no sound comes out and no events fire. To play sound, decode it with [Web Audio](/web-platform/web-audio/): `fetch` the file, `decodeAudioData`, and play an `AudioBufferSourceNode`. [`navigator.mediaSession`](/web-platform/navigator-media-session/) shows what the app plays in the system's media controls; it is not tied to any element, so set it yourself.

## Compared with the browser

- No native controls, picture-in-picture, `<source>` or `<track>` children, MSE, EME or `requestVideoFrameCallback`.
- `play()` never rejects, `load()` does nothing, and there is no autoplay policy.
- `autoplay`, `muted` and `loop` stay live, and `volume`, `playbackrate` and `currenttime` are extra attributes.
- `error`, `buffered` and `seekable` are plain objects.
- A hidden video keeps playing; a detached one pauses.
- `<audio>` plays nothing.

## See also

- [`<video>` in the web-platform reference](/web-platform/video/): examples and edge cases.
- [`<sk-surface>`](/elements/sk-surface/): video from your own native player, such as libmpv.
- [Features and size](/embedding/features/): the `video` feature.
