# Vendored libvpx

| | |
| --- | --- |
| upstream | <https://chromium.googlesource.com/webm/libvpx> (mirror <https://github.com/webmproject/libvpx>) |
| release | **v1.17.0** "Yellowbilled Duck" (2026-08-07), commit `6df3ec34557879fff673706f4a1d9fbd0f3a6f0e` |
| tarball | `https://github.com/webmproject/libvpx/archive/refs/tags/v1.17.0.tar.gz` |
| sha256 | `1020f184046187baa2985dbde38e0691f49c44088bca7a1842b0236c6081dc0a` |
| licence | BSD 3-Clause (`libvpx/LICENSE`) plus Google's Additional IP Rights Grant for the WebM codecs (`libvpx/PATENTS`); contributors in `libvpx/AUTHORS` |

`libvpx/` is **pristine**: every file in it is byte-for-byte upstream, and
nothing in it is edited. What this crate changes about libvpx it changes from
outside: the generated headers under `config/`, three C files of its own
under `csrc/`, and which files `build.rs` compiles. `tools/vendor-libvpx.ts`
rewrites `libvpx/` and `config/` from the release; a clean `git status` after
running it is the audit.

What it is for: the VP9 video stickers and video emoji of `<vn-sticker>`
(`crates/vn-sticker/src/video/`). Media Foundation cannot play them: its only
VP9 decoder is a Store package the user can remove, and it drops the alpha
stream (W0, `research/reports/m26/stickers-1.md`). libvpx is what Chromium
decodes the same files with.

## What is vendored

| what | |
| --- | --- |
| `sources.txt` | the 53 C files `build.rs` compiles: 44 for every target, 9 x86 intrinsics files |
| the headers those reach | 81, found by following `#include "..."` from the sources and from the generated headers |
| `LICENSE`, `PATENTS`, `AUTHORS` | the licence terms, next to the code they cover |

137 files, 1.5 MB, out of the release's 27 MB. **Not** vendored:

- VP8, the VP9 encoder, every encoder-side DSP file, postprocessing.
- The assembly (`*.asm`): the build has no nasm/yasm (below).
- Other architectures' code and headers (`arm/`, `mips/`, `ppc/`,
  `loongarch/`, `vpx_ports/arm*.h` and friends): the `generic` configuration
  is plain C.
- `vpx_mem/vpx_mem.c`: `csrc/vn_vpx_mem.c` replaces it (below).
- The build system (`configure`, `*.mk`, `build/`), tools, examples, tests,
  test vectors, `third_party/`.

## The build

`build.rs` compiles with `cc` from checked-in headers. No `configure`,
`make`, nasm or yasm runs at build time, and nothing is fetched. Two
configurations, both made by libvpx's own `configure` with
`--disable-vp8 --disable-vp9-encoder --enable-vp9-decoder --disable-multithread --size-limit=4096x4096`
(plus the usual `--disable-examples/tools/docs/unit-tests/webm-io/libyuv/postproc`):

| `config/` | target | what it is |
| --- | --- | --- |
| `x86_64-msvc/` | `configure --target=x86_64-win64-vs17` | x86_64 with MSVC: SSE2…AVX2 intrinsics, runtime CPU detection |
| `generic/` | `configure --target=generic-gnu` | every other target: C only |
| `vpx_version.h` | `build/make/version.sh` | shared |

`VN_VPX_CONFIG=generic` forces the C build on x86_64 MSVC as well: the tests
use it as the reference (below).

The three choices worth knowing:

- **`--disable-multithread`.** Every decoder runs on its caller's thread with
  `threads = 1`; the sticker worker pool is the parallelism. In this
  configuration libvpx's `once()` is a plain flag, so `src/lib.rs` runs
  libvpx's process-wide set-up (`vp9_rtcd`, `vpx_dsp_rtcd`, `vpx_scale_rtcd`,
  `vp9_init_intra_predictors`) under a `std::sync::Once` before any decoder
  decodes.
- **`--size-limit=4096x4096`.** A frame header claiming more is refused
  before anything is allocated (the sticker contract calls such a file
  unsupported).
- **Always optimised, `NDEBUG`.** `build.rs` compiles libvpx at `/O2`
  (`-O2`) with no debug info in every cargo profile: a debug build of the
  decoder is many times slower and nobody steps through it. `NDEBUG` makes
  it a release build of libvpx: its `assert`s are internal invariants, and a
  damaged file must not abort the process through one.

### No nasm: the rtcd patch and the asm shim

libvpx's x86 SIMD is part intrinsics (`*_sse2.c`, `*_ssse3.c`, `*_avx2.c`),
part nasm assembly. This crate builds the intrinsics only. Two things in
libvpx assume the assembly is there, and both are handled outside
`libvpx/`:

1. **47 rtcd entries exist only as assembly** (`config/asm-only.txt`): the
   intra predictors' SSE2/SSSE3 versions, `vpx_convolve_copy_sse2`,
   `vpx_convolve_avg_sse2` and `vpx_iwht4x4_16_add_sse2`. The vendor tool
   points each back at what a CPU without it would run: in
   `config/x86_64-msvc/vpx_dsp_rtcd.h` a `#define NAME NAME_sse2` becomes
   `#define NAME NAME_c` (34 lines) and an
   `if (flags & HAS_SSSE3) NAME = NAME_ssse3;` dispatch line is dropped (13
   lines). The header says so at its top. The tool fails if an entry stops
   matching exactly one line, so a version bump cannot leave the list wrong
   silently; a bump that adds an asm-only entry fails to link with that
   name unresolved, and it goes into the list.
2. **46 assembly helpers are called by the intrinsics code**
   (`csrc/vn_vpx_asm_shim.c`): `vpx_filter_block1d{4,8,16}_{h,v}{2,8}[_avg]_{sse2,ssse3}`,
   the 1-D filters that libvpx's SIMD convolutions (`FUN_CONV_1D/2D` in
   `vpx_dsp/x86/convolve.h`) hand some widths, tap counts and the averaging
   (compound prediction) variants to, and `vpx_clear_system_state` (`emms`;
   no MMX code is linked, so a no-op). The SSSE3 8-tap helpers an SSSE3 or
   AVX2 CPU reaches are built from libvpx's own SSSE3 intrinsics (16 wide is
   two 8-wide halves; an averaging helper filters into a scratch block and
   then `pavgb`s it into the destination, as the asm does). The rest (the
   bilinear 2-tap helpers, which libvpx's encoder never selects; the 4-wide
   vertical 8-tap; the SSE2 8-tap helpers only a CPU without SSSE3 reaches)
   are `vpx_convolve8_c`'s arithmetic, scalar.

Measured in W0 against the full nasm build: the intrinsics-only decoder is
4 % slower at 512² (0.445 against 0.428 ms a frame, colour and alpha) and
decodes bit-exactly the same.

### The allocator: `csrc/vn_vpx_mem.c`

libvpx's `vpx_memalign`/`vpx_malloc`/`vpx_calloc`/`vpx_free`, with
`vpx_mem.c`'s arithmetic and overflow guard, plus a small header on each block
naming the decoder that paid for it. `Decoder::memory()` is therefore exact,
and `Decoder::set_memory_limit()` can cap a decoder: past the cap an
allocation fails the way an out-of-memory does inside libvpx, which handles
it. The account is chosen by the calling thread (`src/lib.rs` brackets each
libvpx call); with `--disable-multithread` every allocation happens on that
thread, inside that call.

## Verified

`cargo test -p vn-vpx-sys` (and `--features shim-stats` to see the counts):

- **Every shim helper against libvpx's C** (`vpx_convolve8_{,avg_}{horiz,vert}_c`):
  every phase of every VP9 kernel table it can be handed, heights 1 to 100,
  noise with runs of 0 and 255.
- **Every SIMD convolution against its C**: `vpx_convolve8{,_avg}{,_horiz,_vert}`
  for SSE2, SSSE3 and AVX2, every kernel table, every block size 4…64.
- **The decoder at every SIMD level against the pure-C hash**:
  `testdata/compound.ivf` (5 KB, 48 frames, 8 hidden alt-ref frames, sub-pixel
  motion and compound prediction) decoded with libvpx's dispatch set as a CPU
  with only SSE2, with SSSE3, with AVX2, and with the rtcd C fallbacks, each
  in a child process. All four give the hash of `VN_VPX_CONFIG=generic` (pure
  C), which is also the hash of ffmpeg's native VP9 decoder and of ffmpeg's
  libvpx for the same file. With `shim-stats` the run shows the clip reaching
  the SSE2 8-tap shims (~5400 calls), the SSSE3 8-tap and 8-tap averaging
  shims (~2400 and ~480) and, at AVX2, the 4/8-wide averaging shims (~200).
- Damaged input (noise, every prefix of the keyframe, bit flips through the
  whole stream): errors, never a crash, and the decoder decodes the next
  keyframe.

## Two libvpx behaviours the wrapper works around

- **`VPX_SIMD_CAPS` does nothing on Windows.** `vpx_ports/x86.h` replaces
  `getenv` with `NULL` under `WINAPI_FAMILY_PARTITION(WINAPI_FAMILY_APP)`,
  which a current Windows SDK defines as always true. So no environment
  variable can change the dispatch (good for production); the SIMD-level test
  sets libvpx's dispatch pointers itself instead.
- **A failed frame can leak a frame buffer.** When a frame fails after its
  header has claimed its reference slots (a truncated keyframe, say),
  `release_fb_on_decoder_exit` gives back the old references but not the new
  frame's claims. A dozen such errors and the context has no free buffer,
  even for a good keyframe ("Unable to find free frame buffer"). So
  `Decoder::decode` starts from a fresh libvpx context after any error; the
  next frame needs to be a keyframe, which libvpx would have required anyway.

## Bumping the version

1. Edit `VERSION`, `RELEASE_URL` and `SHA256` in `tools/vendor-libvpx.ts`.
2. From Git Bash (configure is a bash script; rtcd.pl needs perl; the generic
   configure probes `clang`, which stands in for gcc):
   ```
   bun crates/vn-vpx-sys/tools/vendor-libvpx.ts
   ```
   It downloads and checks the tarball, runs `configure` for both
   configurations, generates the rtcd and version headers, applies
   `config/asm-only.txt`, and rewrites `libvpx/` from `sources.txt` and the
   include closure. `--src <extracted release>` skips the download.
3. If the link fails on an unresolved `*_sse2`/`*_ssse3`/`*_avx2` that rtcd
   names: add it to `config/asm-only.txt` and re-run. On an unresolved
   `vpx_filter_block1d*`: add it to `csrc/vn_vpx_asm_shim.c` and to the
   helper test. On a missing source: add it to `sources.txt`.
4. `cargo test -p vn-vpx-sys --features shim-stats`, and the same with
   `VN_VPX_CONFIG=generic` (in its own `CARGO_TARGET_DIR`): both must print
   the same clip hash, and `it_is_the_vendored_release` checks the version.
5. Update the table at the top.
