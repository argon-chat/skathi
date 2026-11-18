---
title: "<img>"
label: "<img>"
metaTitle: "<img>: how Skathi loads, decodes and scales images"
description: How <img> resolves its source, which formats it decodes and on which thread, when load fires, how decodes are shared and shrunk to the size shown, and which attributes and CSS apply.
section: Elements
order: 7
---

Avatars in a member list, thumbnails in a file browser, screenshots in a changelog, a photo the user just picked: `<img>` works as on the web. Give it a `src`, size it with CSS, listen for `load` and `error`.

What Skathi adds is underneath, and it matters most when there are many images. Decoding runs on a background thread, so a page of photos does not stall typing or scrolling. One decode is shared by every element that shows the same source, so the same avatar in a hundred messages is decoded once. An image drawn much smaller than its pixels, such as a camera photo shown as a thumbnail, is drawn from a filtered, smaller copy, and after a while the full decode can be dropped to save memory.

## An avatar with a fallback

A user's avatar URL can be missing or broken. `error` fires once per `src`, so swap in initials when it does. Size and round the image in CSS; `object-fit: cover` crops a non-square picture to the circle.

```vue title="Avatar.vue" framework="vue"
<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{ src: string; name: string }>();
const broken = ref(false);
watch(
  () => props.src,
  () => (broken.value = false),
);
</script>

<template>
  <img v-if="!broken" class="avatar" :src="src" alt="" @error="broken = true" />
  <span v-else class="avatar initials">{{ name.slice(0, 1) }}</span>
</template>

<style scoped>
.avatar {
  width: 40px;
  height: 40px;
  border-radius: 50%;
  object-fit: cover;
}
.initials {
  display: flex;
  align-items: center;
  justify-content: center;
  background: #8e4ec6;
  color: white;
}
</style>
```

```svelte title="Avatar.svelte" framework="svelte"
<script lang="ts">
  let { src, name }: { src: string; name: string } = $props();
  let broken = $state(false);
  $effect(() => {
    void src;
    broken = false;
  });
</script>

{#if !broken}
  <img class="avatar" {src} alt="" onerror={() => (broken = true)} />
{:else}
  <span class="avatar initials">{name.slice(0, 1)}</span>
{/if}

<style>
  .avatar {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    object-fit: cover;
  }
  .initials {
    display: flex;
    align-items: center;
    justify-content: center;
    background: #8e4ec6;
    color: white;
  }
</style>
```

An empty `src` fires `error` too, while no `src` attribute at all fires nothing. A result for a `src` that has since changed is dropped, so a quick change of user never shows the old picture's error.

## Preview a picked image

A picked file becomes an image through a `blob:` URL. Skathi reads a picked file from its path rather than copying it into memory first. Revoke the old URL when a new file replaces it.

```vue title="PhotoPreview.vue" framework="vue"
<script setup lang="ts">
import { onUnmounted, ref } from "vue";

const url = ref("");

function picked(e: Event) {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (url.value) URL.revokeObjectURL(url.value);
  url.value = file ? URL.createObjectURL(file) : "";
}
onUnmounted(() => url.value && URL.revokeObjectURL(url.value));
</script>

<template>
  <input type="file" accept="image/*" @change="picked" />
  <img v-if="url" class="preview" :src="url" alt="Selected photo" />
</template>

<style scoped>
.preview {
  width: 240px;
  aspect-ratio: 4 / 3;
  object-fit: cover;
  border-radius: 8px;
}
</style>
```

```svelte title="PhotoPreview.svelte" framework="svelte"
<script lang="ts">
  let url = $state("");

  function picked(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (url) URL.revokeObjectURL(url);
    url = file ? URL.createObjectURL(file) : "";
  }

  $effect(() => () => {
    if (url) URL.revokeObjectURL(url);
  });
</script>

<input type="file" accept="image/*" onchange={picked} />
{#if url}
  <img class="preview" src={url} alt="Selected photo" />
{/if}

<style>
  .preview {
    width: 240px;
    aspect-ratio: 4 / 3;
    object-fit: cover;
    border-radius: 8px;
  }
</style>
```

A picked file that changes on disk after it was picked fails to load and fires `error`. A 24-megapixel photo in a 240-pixel box is drawn from a shrunk copy, made on a resampling thread for an image that large.

## Things to watch for

- **No responsive images.** `srcset`, `sizes` and `<picture>` are ignored; there is one `src`.
- **No lazy loading.** Every image loads as soon as its `src` is set. For a long gallery, render only the rows in view.
- **Local `load` fires early.** For an app file, a packaged file, `data:` or `blob:`, `load` fires when the size is known, before the pixels are decoded.
- **SVG in `<img>` is a fixed raster** at twice its own size. For an icon that scales, use [inline `<svg>`](/elements/svg/).
- **No AVIF**, and no color management.

## Attributes and properties

| Read              | What it does                                                                       |
| ----------------- | ---------------------------------------------------------------------------------- |
| `src`             | The image; see [Sources](#sources). Loading starts as soon as it is set.           |
| `alt`             | Drawn in place of the image when the source cannot be resolved, loaded or decoded. |
| `width`, `height` | Size hints in CSS pixels, unitless. CSS wins.                                      |

`srcset`, `sizes`, `loading`, `decoding` and `fetchpriority` are ignored without an advisory. `<picture>` and its `<source>` children choose nothing.

The element object has `complete`, `currentSrc` (the `src` once loaded), `naturalWidth` and `naturalHeight` (0 until loaded), and `decode()`, which resolves on `load` and rejects with `EncodingError` on `error`. `new Image()` is a different object: an image for canvas use, with its own fetch and decode, not an element in the tree.

## Events

`load` and `error` fire asynchronously, once per `src`. An empty `src` fires `error`; no `src` attribute fires nothing. A result for a `src` that has since changed is dropped.

For an app file, a packaged file, `data:` or `blob:`, `load` fires when the image's header has been read and its size is known, before the pixels are decoded; `decode()` resolves at the same point. A file with a valid header and corrupt pixels fires `load` and then shows its `alt` text. For `http:` and `https:`, `load` waits for the full decode.

## Sources

| `src`                            | Loads                                                                                                                        |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| A relative or root-relative path | The app's file. A packaged app reads it from its package; a path not in the package shows as broken, with an advisory.       |
| `app://redjs/…`                  | The same app file: the page's own origin.                                                                                    |
| `https:`, `http:`                | Fetched through the same network stack as `fetch`, without cookies.                                                          |
| `data:`                          | Base64 or percent-encoded.                                                                                                   |
| `blob:`                          | An in-memory Blob is copied. A picked file is read from its path, and fails if the file changed on disk since it was picked. |
| `file:`, an absolute path        | The file on disk.                                                                                                            |

A path that climbs out of the app's root prints an advisory, and any other scheme shows as broken with one.

## Formats

PNG, JPEG, GIF, WebP, BMP, ICO, TIFF and PNM are decoded, and SVG is drawn with resvg. Other formats, such as AVIF, are not. Animated GIF and WebP play, looping, while the window is active and the system does not ask for reduced motion; an animated PNG shows its first frame. EXIF orientation is applied. Images are decoded to 8-bit RGBA, without color management.

For `data:` and `blob:` sources the format comes from the MIME type, or from the first bytes when there is none; ICO and TIFF need the MIME type.

An SVG in `<img>` is rasterized once, at twice its own size, whatever size it is shown at, so it blurs when shown larger than that. SVG text uses system fonts, not the app's `@font-face` fonts.

## Decoding, sharing and memory

Decoding runs on a background thread; reading a local file's header to learn its size runs on the UI thread. While an image loads, the element paints nothing.

One decode serves every element that shows the same source. A decode no element shows is freed after 30 seconds.

An image drawn below 0.8 of its pixel size is drawn from a filtered, smaller copy (box steps, then Lanczos), made on the UI thread for images up to about 1.3 megapixels and on a resampling thread for larger ones. After two seconds at that size, a still image may drop its full decode and keep only the small copy; drawing it larger again restores the full decode. Animated images are never reduced. A pop-out window with [`fullResolution`](/window-api/sk-window-full-resolution/) keeps full decodes.

## Size and CSS

An unsized `<img>` takes its natural size in CSS pixels once loaded, and one set side takes the other from the natural ratio or CSS [`aspect-ratio`](/css/aspect-ratio/).

- [`object-fit`](/css/object-fit/) defaults to `fill`, as in browsers; [`object-position`](/css/object-position/) applies.
- [`border-radius`](/css/border-radius/) clips the image.
- [`image-rendering`](/css/image-rendering/) is not supported; images are always filtered.

## Canvas

`drawImage(img)` draws an image the element has loaded: one still loading draws nothing, a broken one throws `InvalidStateError`. Except for `http:` and `https:` images, canvas decodes the pixels itself, on the UI thread, and it reads PNG, JPEG, WebP, BMP, GIF and SVG only: a TIFF or ICO the element shows cannot be drawn. A canvas is never tainted.

## Compared with the browser

- No responsive images: `srcset`, `sizes` and `<picture>` are ignored.
- No lazy loading: `loading="lazy"` loads at once.
- `load` for a local image fires at the header, not after the full decode.
- SVG in `<img>` is a fixed-size raster.
- No AVIF, no color management.

## See also

- [Inline `<svg>`](/elements/svg/).
- [`<canvas>`](/elements/canvas/) and [Canvas 2D in the web-platform reference](/web-platform/canvas-2d/).
- [Best practices](/guides/best-practices/).
