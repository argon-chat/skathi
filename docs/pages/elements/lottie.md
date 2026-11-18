---
title: "<lottie>"
label: "<lottie>"
metaTitle: "<lottie>: play Lottie animations"
description: Play a Lottie (Bodymovin) animation from JSON or a .tgs file. Attributes, events, sources, how frames are rasterized off the UI thread, sizing and CSS, and what differs from lottie-web.
section: Elements
order: 3
---

`<lottie>` plays a Lottie animation, the JSON format After Effects exports through Bodymovin, or a `.tgs` file, which is the same JSON gzipped. Its attributes and events copy lottie-web's names, and its defaults copy `<lottie-player>`'s. There is no npm package to install and no player script: the element is drawn by Skathi.

For many small animated stickers on screen at once, use [`<sk-sticker>`](/elements/sk-sticker/), which shares decodes and frames between copies.

## A minimal example

```vue
<script setup lang="ts">
import { ref } from "vue";

const playing = ref(true);
const loops = ref(0);
</script>

<template>
  <lottie class="hero" src="assets/hello.tgs" loop :autoplay="playing" @loopComplete="loops++" />
  <button @click="playing = !playing">{{ playing ? "Pause" : "Play" }}</button>
</template>

<style scoped>
.hero {
  width: 160px;
  height: 160px;
}
</style>
```

```svelte
<script lang="ts">
  let playing = $state(true);
  let loops = $state(0);
</script>

<lottie class="hero" src="assets/hello.tgs" loop autoplay={playing} onloopcomplete={() => loops++}></lottie>
<button onclick={() => (playing = !playing)}>{playing ? "Pause" : "Play"}</button>
```

Vue compiles `<lottie>` as a custom element, so no registration is needed.

## Attributes

| Attribute         | Values                                                                                    | Default             |
| ----------------- | ----------------------------------------------------------------------------------------- | ------------------- |
| `src`             | The animation's URL or path; see [Sources](#sources). Changing it reloads from the start. | none                |
| `autoplay`        | Boolean. Turning it on plays, turning it off pauses and holds the current frame.          | off                 |
| `loop`            | Absent: play once. Present or `"true"`: loop forever. A number `n`: play `n` more times.  | play once           |
| `speed`           | A multiplier on the animation's frame rate. Below 0 counts as 0, which stops it.          | `1`                 |
| `direction`       | `"-1"` or `"reverse"` plays backwards.                                                    | forwards            |
| `mode`            | `"bounce"` reverses at each end instead of jumping back to the start.                     | normal              |
| `segment`         | Two source frame numbers, `"30,59"` or `"30 59"`: play only that range.                   | the whole animation |
| `objectfit`       | `fill`, `contain`, `cover`, `scale-down` or `none`. The CSS `object-fit` property wins.   | `contain`           |
| `width`, `height` | Size hints in CSS pixels, unitless. CSS wins.                                             | none                |

A boolean attribute counts as on when present, unless its value is `"false"`, so `:autoplay="playing"` in Vue and `autoplay={playing}` in Svelte work as expected. `loop` counts the way lottie-web does: `loop="2"` plays three times.

`autoplay` is a transport control, not a one-time hint: it acts every time its value changes. That is how an app plays and pauses: bind it to state. The element has no `play()`, `pause()`, `stop()` or seek methods and no `currentTime` or `duration` properties; the only reflected property is `src`.

These lottie-web and `<lottie-player>` attributes are not supported and print an advisory naming the alternative: `renderer`, `background` (use CSS `background`; the raster is transparent), `controls` (build controls in Vue or Svelte), `hover` (bind `autoplay` to pointer events), `intermission`, `count` (use `loop="n"`), `preserveAspectRatio` (use `object-fit`), `webworkers` and `subframe` (rendering is always off the UI thread and subframe).

## Events

| Event          | When                                                     | Carries                                                    |
| -------------- | -------------------------------------------------------- | ---------------------------------------------------------- |
| `enterFrame`   | every frame in which the playhead moved                  | `currentTime`, `totalTime`, `elapsed`, `progress` (0 to 1) |
| `loopComplete` | at each loop boundary, except the one that ends playback | `currentTime`, `totalTime`, `currentLoop`                  |
| `complete`     | playback finishes                                        | `currentTime`, `totalTime`, `loops`                        |

The values are on the event and on `detail`. **`currentTime` and `totalTime` are in frames, not seconds**, as in lottie-web: `currentTime` is the fractional source frame and `totalTime` the animation's length in frames. The events do not bubble, and they are only dispatched to an element with a listener.

Vue turns `@loopComplete` into a listener for `loop-complete`, and Svelte's `onloopcomplete` listens for `loopcomplete`; both are accepted as `loopComplete`, and the same holds for `enterFrame`.

There is no `load` or `error` event. A source that fails to load or parse prints a console warning, and the element stays empty.

## Sources

`src` accepts:

- A path relative to the app, such as `assets/hello.tgs`, or a root-relative one. In a packaged app it is read from the package.
- `https:` and `http:` URLs, fetched through the same network stack as `fetch`, so they show in the devtools Network panel.
- `data:` URLs, base64 or percent-encoded, and `blob:` URLs.
- `file:` URLs and absolute paths.

The format is recognized by its first bytes, not its extension: gzip is read as `.tgs`, anything else as Lottie JSON. dotLottie (`.lottie`, a zip) is not supported.

### Packing

A packaged app carries only the files `sk build` packs. A file in the `assets/` directory is packed by default. Anywhere else, reference it so the build sees it:

- `<lottie src>` with a relative path such as `./hello.json`, in a Vue template, is compiled to an import, like `<img src>`. A `.json` file is imported as its URL, not as the parsed JSON.
- `new URL("./hello.tgs", import.meta.url)` in script packs the file and gives its URL.

A path that is not in the package fails with a warning that says so. Svelte templates do not get the template transform; use `new URL(…, import.meta.url)` there.

## How it is drawn

Each `<lottie>` loads and parses its own copy of the animation. File reads, parsing and drawing run on one dedicated `sk-lottie` thread; in-memory sources (`data:`, `blob:`, packaged files) are copied on the UI thread first. The renderer is [ThorVG](https://www.thorvg.org/), with a pure-Rust fallback. ThorVG draws what it understands, including expressions, and skips the rest. Two Bodymovin idioms ThorVG does not draw are rewritten before parsing, and the rewrite is reported as an advisory.

The playhead lives on the UI thread. Each frame, it advances by the real time elapsed, times the animation's frame rate, times `speed`, and it is never snapped to whole frames. A pause longer than 250 ms counts as 250 ms. The element then asks the worker for that frame at the box's size in device pixels; one request per element is in flight, and a newer one replaces an older one. The finished pixels are written into the element's GPU tile, so a new frame costs a repaint, not a rebuild of the page.

A frame is drawn only while the element is visible: in a shown window and a visible page, and inside the viewport and every clipping ancestor. The playhead keeps advancing while it is hidden or scrolled away, and the events keep firing; an element removed from the document but kept alive, as in `<KeepAlive>`, keeps its clock too. `prefers-reduced-motion` is not read: to stop an animation for users who ask for less motion, bind `autoplay` to the media query.

When the element is removed, its animation, its ThorVG canvas and its GPU tile are released.

## Size and CSS

Before the animation loads the element has no natural size, so an unsized `<lottie>` is 0 × 0 until then. After it loads, an unsized element takes the animation's width and height in CSS pixels, and an element with one side set takes the other from the animation's aspect ratio, or from CSS [`aspect-ratio`](/css/aspect-ratio/). Give the element a size in CSS so the layout does not shift when it loads.

- [`object-fit`](/css/object-fit/) defaults to `contain`, not CSS's initial `fill`. The animation is rasterized at the contained size, so `cover` and `fill` scale that raster up.
- [`object-position`](/css/object-position/) applies.
- The raster is transparent, so the element's `background` shows through.

## Platforms

`<lottie>` behaves the same on Windows, macOS and Linux. It needs the host's `lottie` Cargo feature, on by default; without it the element is an empty box and the first one prints a warning. See [Features and size](/embedding/features/).

## Compared with lottie-web

- `autoplay` and `loop` are off by default.
- Playback is controlled by attributes only; there is no animation object with methods.
- A negative `speed` stops the animation instead of reversing it. Use `direction`.
- `loopComplete` does not fire on the boundary that ends playback.
- There are no `load`, `error`, `DOMLoaded` or `data_ready` events.
- Rendering is always raster: there is no SVG, canvas or HTML renderer to choose, and dotLottie files are not read.

## See also

- [`<sk-sticker>`](/elements/sk-sticker/): many small animations at once, with Lottie or WebM sources.
- [`object-fit`](/css/object-fit/) and [`aspect-ratio`](/css/aspect-ratio/) in the CSS reference.
- [Features and size](/embedding/features/): the `lottie` feature.
