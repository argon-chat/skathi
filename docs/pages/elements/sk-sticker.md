---
title: "<sk-sticker>"
label: "<sk-sticker>"
metaTitle: "<sk-sticker>: many animated stickers at once"
description: Play animated stickers and emoji from Lottie (.json, .tgs) or WebM with alpha, many at once. Copies of one source share a decode, a clock and their frames. Attributes, events, caching, sizing and reduced motion.
section: Elements
order: 4
---

`<sk-sticker>` plays small animations: stickers, animated emoji, reactions. It is built for many of them on screen at once, such as a chat with the same sticker in fifty messages. Every copy of a source shares one download, one decoder, one clock and, at the same size, the same frames. Sources are Lottie (`.json` or `.tgs`) or WebM video with an alpha channel.

For a single animation with full playback control (segments, speed, direction, per-frame events), use [`<lottie>`](/elements/lottie/).

## A minimal example

```vue
<script setup lang="ts">
import { ref } from "vue";

const done = ref(false);
</script>

<template>
  <sk-sticker class="emoji" src="assets/stickers/wave.tgs" />
  <sk-sticker class="emoji" src="assets/stickers/wave.tgs" once @ended="done = true" />
  <sk-sticker class="emoji mono" src="assets/stickers/heart.tgs" tint />
</template>

<style scoped>
.emoji {
  width: 32px;
  height: 32px;
}
.mono {
  color: var(--accent);
}
</style>
```

```svelte
<script lang="ts">
  let done = $state(false);
</script>

<sk-sticker class="emoji" src="assets/stickers/wave.tgs"></sk-sticker>
<sk-sticker class="emoji" src="assets/stickers/wave.tgs" once onended={() => (done = true)}></sk-sticker>
```

## Attributes

| Attribute         | Values                                                                                                                           | Default |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `src`             | The sticker's URL or path; see [Sources](#sources).                                                                              | none    |
| `play`            | `auto`: plays, or shows a still frame when the system asks for reduced motion. `on`: always plays. `off`: shows the first frame. | `auto`  |
| `once`            | Boolean. Plays one loop on its own clock, starting when it first becomes visible, then holds the last frame.                     | off     |
| `tint`            | Boolean. Paints the sticker in the element's CSS `color`, keeping its shape and transparency.                                    | off     |
| `max-fps`         | A frame-rate cap, 1 to 1000. Also spelled `maxFps` or `maxfps`.                                                                  | none    |
| `kind`            | `auto`, `lottie`, `video` or `image`. Only `image` changes anything; the format is otherwise detected.                           | `auto`  |
| `width`, `height` | Size hints in CSS pixels, unitless. CSS wins.                                                                                    | none    |

A boolean attribute counts as on when present, unless its value is `"false"`. An unknown `play` or `kind` value prints an advisory and counts as `auto`.

The element has no JavaScript properties or methods beyond those of any element. `src` is not reflected as a property: set it with Vue's `:src`, Svelte's `src={…}` or `setAttribute`.

## Events

| Event   | When                                                                                                        |
| ------- | ----------------------------------------------------------------------------------------------------------- |
| `load`  | once, when the sticker's first frame is shown. A sticker that is never visible never fires it.              |
| `error` | once per element, when the source cannot be fetched, read or decoded. The reason is printed to the console. |
| `ended` | with `once`, after its single loop. Not fired for a still sticker.                                          |

None of them bubbles.

## Sources

`src` accepts the same URLs as [`<lottie>`](/elements/lottie/#sources): app paths, `https:` and `http:`, `data:`, `blob:`, `file:` and absolute paths. The format is recognized by the first bytes:

- WebM: VP9 with an alpha channel, decoded by libvpx, the same on every platform. It needs the host's `video` Cargo feature as well as `lottie`; without it a WebM sticker fires `error`.
- gzip: a `.tgs` file.
- JSON: a Lottie animation.

Anything else is rejected, including dotLottie files and images, unless `kind="image"` sends the source through the [`<img>`](/elements/img/) pipeline. An image sticker reports no natural size, so give it one in CSS.

A source is fetched once, however many elements show it. An `<sk-sticker src>` in a Vue template is not turned into an import, so keep sticker files under `assets/`, which `sk build` packs by default, or reference them with `new URL("./wave.tgs", import.meta.url)`.

## How it is drawn

All copies of a source share one engine and one clock, so they move together. The clock stops while no copy is visible and playing, and resumes where it stopped. Frames change on a 1/60 s grid, so stickers never make a window present more than 60 times a second. A sticker with `once` has its own clock instead.

Each source and size is a target with its own frames, shared by every copy shown at that size. The size is the picture under `object-fit` in device pixels, with the long side rounded up to steps of 4, 8 or 16 pixels so near sizes share frames, and capped at 1024 pixels for Lottie and at the video's own size for WebM.

Small targets, no more than 64 device pixels on either side, play at up to 30 fps. Their whole loop is cached, compressed, under a shared 24 MB budget; once a loop is cached the source's decoder is freed.

Parsing and drawing run on one `sk-sticker` thread. Local files and packaged files are read on the UI thread when the element first needs them; `http:` and `https:` sources are fetched in the background.

Resources go when they are not used:

- a target no visible copy uses is dropped after 3 seconds;
- a source's decoder and bytes after 15 seconds;
- a sticker's GPU tile after 3 seconds without a visible copy painting it.

At most four WebM decoders larger than 256 × 256 run at once.

## Size and CSS

Before the source loads the element has no natural size; after it loads, an unsized sticker takes the source's size in CSS pixels, and [`aspect-ratio`](/css/aspect-ratio/) applies as on `<img>`. Stickers are usually sized in CSS.

- [`object-fit`](/css/object-fit/): `contain` (the default), `cover` and `fill`. `none` and `scale-down` draw as `contain` with an advisory: a sticker is drawn at the size it is shown, so it has no pixel size to keep.
- `object-position` is not applied.
- [`border-radius`](/css/border-radius/) clips the picture.
- `color` is used with `tint`.

## Reduced motion

With `play="auto"`, the default, a sticker follows the system's reduced-motion setting and shows a still frame while it is on. `play="on"` plays anyway, for a sticker the user asked to play.

## Platforms

`<sk-sticker>` behaves the same on Windows, macOS and Linux. It needs the host's `lottie` Cargo feature, on by default; without it the element is an empty box and the first one prints a warning. See [Features and size](/embedding/features/).

## See also

- [`<lottie>`](/elements/lottie/): one animation, with segments, speed, direction and per-frame events.
- [Features and size](/embedding/features/): the `lottie` and `video` features.
