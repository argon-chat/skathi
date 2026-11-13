# Vendored ThorVG

|          |                                                                                                                            |
| -------- | -------------------------------------------------------------------------------------------------------------------------- |
| upstream | <https://github.com/thorvg/thorvg>                                                                                         |
| release  | **v1.1.0** (2026-07-22)                                                                                                    |
| tarball  | `https://github.com/thorvg/thorvg/archive/refs/tags/v1.1.0.tar.gz`                                                         |
| sha256   | `73943bcf918c9ffdf11acc53052ba5cb60a7047b1cfb2067afeb62e0c4d9417d`                                                         |
| licence  | MIT (`thorvg/LICENSE`); the bundled JerryScript is Apache-2.0 (`thorvg/src/loaders/lottie/jerryscript/jerry-core/LICENSE`) |

`thorvg/` is **pristine** — every file is byte-for-byte upstream. No patches,
no local edits. `git diff` after a fresh `tools/vendor-thorvg.sh` run is the
audit, and that is the whole reason the tree is copied rather than patched.

## Why vendored rather than fetched at build time

`crates/vn-effects/tools/slang.manifest.json` is the repo's fetch-and-pin
precedent, and it is a precedent for a **contributor** tool: `slangc` is
downloaded to author shaders, the generated HLSL is checked in, and
`cargo build` never touches the network. The same rule applied here says the
_build input_ has to be in the tree. A build-script download would also be
exactly the property `research/reports/m10/lottie.md` rejected `rlottie` for
("build script `git clone`s rlottie from GitHub at build time
(non-hermetic)"), so fetching ThorVG the same way would have re-imported the
blocker under a nicer name.

The cost is 7.6 MiB / 532 files in the repo. That buys: offline builds,
reproducible builds, a diffable upgrade path, and no `curl`/`tar`/TLS
dependency on a Windows dev box.

## What is copied

`tools/vendor-thorvg.sh` copies only `*.cpp`, `*.h` and `LICENSE`:

| path                       | why                                                     |
| -------------------------- | ------------------------------------------------------- |
| `inc/thorvg.h`             | public C++ header the whole tree includes               |
| `src/common/`              | containers, math, colour, string, compressor            |
| `src/renderer/`            | scene graph, paint, picture, animation, task scheduler  |
| `src/renderer/cpu_engine/` | the software rasteriser — the only engine we build      |
| `src/bindings/capi/`       | `thorvg_capi.h` + `tvgCapi.cpp`, the ABI we bind        |
| `src/loaders/raw/`         | referenced unconditionally by the C API                 |
| `src/loaders/lottie/`      | the Lottie loader, plus `rapidjson/` and `jerryscript/` |
| `src/loaders/png/`         | lodepng, for Lottie's embedded PNG assets               |
| `src/loaders/jpg/`         | built-in JPEG decoder, for embedded JPEG assets         |
| `src/loaders/sfnt/`        | TTF/OTF, for Lottie text layers backed by a real font   |

**Not** copied, and why:

- `src/renderer/gpu_engine/` — GL and WebGPU. Linking either needs system
  headers and a GPU context; `vn-lottie` composites raster frames through
  gpui's atlas, so the software engine is the whole requirement.
- `src/loaders/svg/`, `src/loaders/webp/`, `src/loaders/external_*/`,
  `src/savers/`, `src/tools/`, `test/` — not reachable from the Lottie path.
  `external_*` link system libpng/libjpeg/libwebp, i.e. exactly the system
  dependency this crate exists to avoid.
- `meson.build` files — the build is cc-rs; keeping them would imply meson
  works, and it is not wired up.

## Bumping the version

1. Edit `VERSION` and `SHA256` in `tools/vendor-thorvg.sh`.
2. `bash crates/vn-thorvg-sys/tools/vendor-thorvg.sh` — it verifies the
   sha256 before unpacking and rewrites `thorvg/` from scratch.
3. Regenerate the checked-in FFI:
   ```
   LIBCLANG_PATH="C:\Program Files\LLVM\bin" \
     cargo run --release --manifest-path crates/vn-thorvg-sys/tools/regen-bindings/Cargo.toml
   ```
4. Update the table above and `THORVG_VERSION_STRING` in `build.rs`.
5. `cargo test -p vn-thorvg-sys` — the first test asserts the compiled
   archive reports the version the bindings were generated from, so a
   half-finished bump fails loudly.

## Config

`build.rs` writes the `config.h` meson would generate. The values track
upstream's `meson_options.txt` defaults so the vendored engine behaves like a
stock `meson setup` build:

| meson option | upstream default           | here                                                     |
| ------------ | -------------------------- | -------------------------------------------------------- |
| `engines`    | `['cpu']`                  | cpu (GPU sources not vendored)                           |
| `loaders`    | `['svg', 'lottie', 'ttf']` | lottie, ttf/otf, png, jpg — **no svg**                   |
| `extra`      | `['lottie_exp', 'openmp']` | `lottie_exp` (feature `expressions`); no OpenMP          |
| `threads`    | `true`                     | feature `threads`, on by default                         |
| `partial`    | `true`                     | on (inert unless a canvas opts into `SMART_RENDER`)      |
| `simd`       | `false`                    | feature `avx`, **off** by default                        |
| `file_io`    | —                          | on; harmless, and `tvgCapi.cpp` references the path APIs |
| `static`     | `false`                    | always static (`-DTVG_STATIC`)                           |
