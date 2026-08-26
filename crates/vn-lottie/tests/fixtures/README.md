# Test fixtures

## Provenance

`mask.json`, `precomp.json`, `rectangle.json`, `solid-layer.json`, `star.json`
and `trim-group.json` come from [`LottieFiles/test-files`][test-files], whose
README states that *"everything in `./data/` is public domain (CC0)"*. They are
committed unmodified, byte for byte. Each was chosen for the renderer feature it
exercises:

| file | exercises |
| --- | --- |
| `star.json` | polystar shape, animated transform |
| `rectangle.json` | animated rectangle, non-square canvas (100×50) |
| `trim-group.json` | trim-path shape modifier |
| `precomp.json` | precomposition layer (`ty: 0`) |
| `mask.json` | layer mask |
| `solid-layer.json` | solid layer (`ty: 1`) — the one `normalize` rewrites |

Source paths under `data/`, at `main`:
`shapes/star.json`, `shapes/rectangle.json`, `shape-modifiers/trim-group.json`,
`layers/precomp.json`, `layers/mask.json`, `layers/solid-layer.json`.

Each has a sibling `*-00.png` golden render upstream, useful if these tests ever
grow image-diff assertions.

## `sticker.tgs`

Generated from `star.json` (so it inherits CC0) by scaling the layer transforms
onto a 512×512 canvas, setting `"tgs": 1`, and gzipping — i.e. exactly what a
Telegram sticker is. It satisfies Telegram's submission constraints (512×512,
60 fps, 2 s, 494 bytes ≪ 64 KB), which makes it a faithful `.tgs` test and not
just "a gzipped JSON".

[test-files]: https://github.com/LottieFiles/test-files
