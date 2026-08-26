# Test fixtures

Every claim in this file is asserted somewhere in `tests/playback.rs` —
`the_animated_list_matches_what_the_documents_animate` in particular walks each
document for animated properties and fails if the "animates" column below
drifts from the files. That test exists because this README used to describe
`star.json` as an "animated transform" when the document has no animated
property at all, and nothing contradicted it for a whole milestone: the
transport tests never needed a moving file.
`research/reports/m10/integration.md` §2.1 is where it finally surfaced, after a
demo capture came back byte-identical and looked like a renderer bug.

## Provenance

`mask.json`, `precomp.json`, `rectangle.json`, `solid-layer.json`, `star.json`
and `trim-group.json` come from [`LottieFiles/test-files`][test-files], whose
README states that *"everything in `./data/` is public domain (CC0)"*. They are
committed unmodified, byte for byte. Each was chosen for the renderer feature it
exercises:

| file | exercises | animates |
| --- | --- | --- |
| `star.json` | polystar shape, **static** transform | no |
| `rectangle.json` | rectangle, non-square canvas (100×50) | no |
| `trim-group.json` | trim-path shape modifier | no |
| `precomp.json` | precomposition layer (`ty: 0`) | no |
| `mask.json` | layer mask | no |
| `solid-layer.json` | solid layer (`ty: 1`) — the one `normalize` rewrites | no |
| `position-linear.json` | linear position keyframes | **yes** |
| `position-hold.json` | hold keyframes (steps at frame 60 of 61) | **yes** |
| `terminator-keyframe.json` | Bodymovin terminator keyframe (`{"t": n}`) | **yes** |
| `sticker.tgs` | the `.tgs` gzip wrapper and `"tgs": 1` marker | no |
| `spinning-star.tgs` | an *animated* `.tgs` | **yes** |

The upstream feature fixtures are single-frame **by design** — they isolate the
rendering of one feature, not its animation — so the four animated files are
what the motion assertions run against.

Source paths under `data/`, at `main`:
`shapes/star.json`, `shapes/rectangle.json`, `shape-modifiers/trim-group.json`,
`layers/precomp.json`, `layers/mask.json`, `layers/solid-layer.json`.

Each has a sibling `*-00.png` golden render upstream, useful if these tests ever
grow image-diff assertions.

## `sticker.tgs` — the wrapper, not the motion

Generated from `star.json` (so it inherits CC0) by scaling the layer transforms
onto a 512×512 canvas, setting `"tgs": 1`, and gzipping — i.e. exactly what a
Telegram sticker is. It satisfies Telegram's submission constraints (512×512,
60 fps, 2 s, 494 bytes ≪ 64 KB), which makes it a faithful `.tgs` test and not
just "a gzipped JSON".

It is **static**, because `star.json` is. It proves the gunzip, the `"tgs"`
marker and the constraint arithmetic, and nothing about playback.

## `spinning-star.tgs` — the motion

The same CC0 polystar and fill from `star.json`, radii scaled ×4 onto a 512×512
canvas, plus an **authored** 0→360° rotation and a 78 %→104 %→78 % scale pulse
over 120 frames at 60 fps, with `"tgs": 1`. 555 bytes, so still a valid Telegram
submission. Both the derivation and the added animation are CC0.

Byte-identical to `apps/hello/assets/sticker.tgs`, which is where it was
authored (`research/reports/m10/integration.md` §2.1) — the demo page needed a
sticker that visibly moved for exactly the reason the crate did.

[test-files]: https://github.com/LottieFiles/test-files
