# hello app assets

## `sticker.tgs`

A Telegram sticker: gzipped Bodymovin JSON with the `"tgs": 1` marker,
512×512, 60 fps, 120 frames (2 s), 555 bytes — inside every one of Telegram's
submission constraints, so it is a faithful `.tgs` and not just "a gzipped
JSON". Used by `src/pages/MotionPage.vue` to demo `<lottie>`.

**Provenance.** The polystar shape and its fill colour are taken verbatim from
`crates/vn-lottie/tests/fixtures/star.json`, which comes from
[`LottieFiles/test-files`](https://github.com/LottieFiles/test-files) and is
public domain (CC0). The radii are scaled ×4 onto the 512×512 canvas, and the
group transform carries an **authored** 0→360° rotation plus a 78%→104%→78%
scale pulse over the 120 frames. Both derivation and addition are CC0.

The rotation is the point: `star.json` — and the `sticker.tgs` in the crate's
fixtures that was derived from it — have **no animated properties at all**
(verified by walking the document for `"a": 1`), so they can demonstrate the
transport but never show motion. The crate's fixture README describing
`star.json` as an "animated transform" is inaccurate.

## `clip-1080p.mp4`, `clip-1080p.poster.png`, `unsupported.mp4`

The `<video>` demo's media (`src/pages/MediaPage.vue`).

**Provenance: generated locally with ffmpeg from synthetic sources.** No third
party content, no licence to track. The exact commands are below, so the clips
are reproducible rather than binary blobs of unknown origin.

### `clip-1080p.mp4` — 1920×1080, 30 fps, 12 s, H.264 High + AAC-LC, 344 KB

```
ffmpeg -y \
  -f lavfi -i "smptehdbars=size=1920x1080:rate=30:duration=12" \
  -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=12" \
  -vf "drawbox=x='(iw-260)*mod(t\,4)/4':y=40:w=260:h=140:color=white@0.95:t=fill,\
drawtext=text='%{pts\:hms}':fontsize=140:fontcolor=white:box=1:boxcolor=black@0.75:\
boxborderw=18:x=(w-text_w)/2:y=(h-text_h)/2" \
  -c:v libx264 -profile:v high -pix_fmt yuv420p -crf 30 -g 30 -tune stillimage \
  -c:a aac -b:a 64k -movflags +faststart clip-1080p.mp4
```

Everything in it is there to be _verifiable_:

- **H.264 High profile 8-bit + AAC-LC in MP4** is the guaranteed inbox baseline —
  the one combination that decodes on a clean Windows install with no Store
  codec extension (`crates/vn-video/CONTRACT.md` §8). The demo must work on the
  machine that has nothing.
- **A burnt-in `HH:MM:SS.mmm` timecode**, so a screenshot proves _which frame_
  is on screen. Seek correctness is then a thing you can read rather than infer.
- **A white bar sweeping left→right on a 4 s cycle**, so motion is visible even
  in a crop that misses the timecode.
- **SMPTE bars** as the background because they are static and compress to
  nothing: 1080p for twelve seconds in 344 KB.
- **A real 330 Hz audio track**, because the whole argument for
  `IMFMediaEngine` over a bare decoder is that the OS owns the audio clock. A
  silent clip would never exercise it.
- **1080p** specifically: the memory and frame-time numbers in the m11/m12
  reports are quoted per 1080p stream, and a demo at 480p would not test them.

`clip-1080p.poster.png` is frame 0 scaled to 640 px wide (`ffmpeg -i
clip-1080p.mp4 -ss 0 -frames:v 1 -vf scale=640:-1`), used for the `poster`
attribute.

### `unsupported-codec.mkv` — FFV1 in Matroska, 192 KB

```
ffmpeg -y -f lavfi -i "testsrc2=size=320x180:rate=15:duration=2"   -c:v ffv1 -level 3 -an unsupported-codec.mkv
```

**Deliberately undecodable, and chosen after measuring.** The first attempt was
Theora in an Ogg container wearing an `.mp4` extension — which turned out to
**play**: this machine has `Microsoft.WebMediaExtensions`, which adds an Ogg
source that is not in Microsoft's documented inbox list. MPEG-4 Part 2 in AVI
plays too. FFV1 is the case that actually fails, and it fails in the _more_
instructive way: Media Foundation reads the Matroska container perfectly well
and has no decoder for the codec inside it, so the error is genuinely "this
machine cannot decode this", not "I could not open the file".

The demo uses it to show the `error` path producing an advisory that names what
this machine can and cannot decode, and what to re-encode to.
