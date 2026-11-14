/*
 * sk-vpx-sys: the assembly-only helpers libvpx's x86 intrinsics call, built
 * without nasm.
 *
 * libvpx's SIMD convolutions (FUN_CONV_1D/FUN_CONV_2D in
 * vpx_dsp/x86/convolve.h, instantiated by vpx_subpixel_4t_intrin_sse2.c,
 * vpx_subpixel_8t_intrin_ssse3.c and vpx_subpixel_8t_intrin_avx2.c) are
 * intrinsics, but for some widths, tap counts and the averaging (compound
 * prediction) variants they call 1-D helpers that exist only in
 * vpx_subpixel_8t_{sse2,ssse3}.asm and vpx_subpixel_bilinear_{sse2,ssse3}.asm.
 * This file defines those 45 symbols, plus vpx_clear_system_state (emms in
 * vpx_ports/emms_mmx.asm), with the same results:
 *
 * - The SSSE3 8-tap helpers the AVX2/SSSE3 paths reach are built from the
 *   SSSE3 intrinsics libvpx already has (vpx_filter_block1d{4,8}_h8 and
 *   vpx_filter_block1d8_v8 _intrin_ssse3): 16 wide is two 8-wide halves, and
 *   an averaging helper filters into a scratch block and then averages it into
 *   dst with pavgb, as the asm does. Every libvpx SIMD helper is bit-exact with
 *   vpx_convolve8_c, so the composition is too.
 * - The rest (the 2-tap bilinear helpers, the 4-wide vertical 8-tap, and the
 *   SSE2 8-tap helpers that only a CPU without SSSE3 reaches) are the scalar
 *   loop below, which is vpx_convolve8_c's arithmetic.
 *
 * src/lib.rs's tests compare every helper with libvpx's own C convolutions,
 * for every VP9 kernel and phase.
 *
 * Semantics of a filter8_1dfunction (vpx_dsp/x86/convolve.h): `h` rows of
 * `w` output pixels. A horizontal helper gets `src` at the output pixel and
 * reads x-3..x+4 (8-tap) or x..x+1 (2-tap); a vertical helper gets `src`
 * already moved up by taps/2-1 rows and reads the rows below it. The 2-tap
 * kernels are the bilinear ones, whose only non-zero taps are filter[3] and
 * filter[4]. The averaging variants write (dst + result + 1) >> 1.
 */
#include <emmintrin.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "./vpx_config.h"
#include "vpx_dsp/x86/convolve.h"
#include "vpx_ports/mem.h"

#if !VPX_ARCH_X86_64
#error "sk_vpx_asm_shim.c is for the x86_64-msvc configuration only"
#endif

/* Defined in vpx_dsp/x86/vpx_subpixel_8t_intrin_ssse3.c (x86_64 only). */
filter8_1dfunction vpx_filter_block1d4_h8_intrin_ssse3;
filter8_1dfunction vpx_filter_block1d8_h8_intrin_ssse3;
filter8_1dfunction vpx_filter_block1d8_v8_intrin_ssse3;

/* No MMX code is linked (every helper here and in libvpx's intrinsics is SSE2
   or later), so there is no x87/MMX state to clear. */
void vpx_clear_system_state(void) {}

static uint8_t clip_pixel(int v) { return (uint8_t)(v < 0 ? 0 : v > 255 ? 255 : v); }

/* vpx_convolve8_c's arithmetic for one 1-D pass: FILTER_BITS = 7, rounded,
   clipped, then averaged into dst when `avg`. */
static void convolve_c(const uint8_t *src, ptrdiff_t src_stride, uint8_t *dst, ptrdiff_t dst_stride,
                       uint32_t h, const int16_t *filter, int w, int vertical, int taps, int avg) {
  const ptrdiff_t step = vertical ? src_stride : 1;
  for (uint32_t y = 0; y < h; y++) {
    for (int x = 0; x < w; x++) {
      const uint8_t *s = src + x;
      int sum = 0;
      if (taps == 8) {
        const uint8_t *p = vertical ? s : s - 3;
        for (int k = 0; k < 8; k++) sum += p[k * step] * filter[k];
      } else {
        sum = s[0] * filter[3] + s[step] * filter[4];
      }
      const uint8_t r = clip_pixel((sum + 64) >> 7);
      dst[x] = avg ? (uint8_t)((dst[x] + r + 1) >> 1) : r;
    }
    src += src_stride;
    dst += dst_stride;
  }
}

/* The SSSE3 8-tap filters the asm has at 16 wide, as two 8-wide intrinsic
   halves. */
static void h8_16_ssse3(const uint8_t *src, ptrdiff_t src_stride, uint8_t *dst, ptrdiff_t dst_stride,
                        uint32_t h, const int16_t *filter) {
  vpx_filter_block1d8_h8_intrin_ssse3(src, src_stride, dst, dst_stride, h, filter);
  vpx_filter_block1d8_h8_intrin_ssse3(src + 8, src_stride, dst + 8, dst_stride, h, filter);
}

static void v8_16_ssse3(const uint8_t *src, ptrdiff_t src_stride, uint8_t *dst, ptrdiff_t dst_stride,
                        uint32_t h, const int16_t *filter) {
  vpx_filter_block1d8_v8_intrin_ssse3(src, src_stride, dst, dst_stride, h, filter);
  vpx_filter_block1d8_v8_intrin_ssse3(src + 8, src_stride, dst + 8, dst_stride, h, filter);
}

/* `w` x `n` bytes of `tmp` (rows of 16) averaged into dst, as pavgb. */
static void average_into(uint8_t *dst, ptrdiff_t dst_stride, const uint8_t *tmp, int w, uint32_t n) {
  for (uint32_t y = 0; y < n; y++, dst += dst_stride, tmp += 16) {
    const __m128i t = _mm_load_si128((const __m128i *)tmp);
    if (w == 16) {
      const __m128i d = _mm_loadu_si128((const __m128i *)dst);
      _mm_storeu_si128((__m128i *)dst, _mm_avg_epu8(d, t));
    } else if (w == 8) {
      const __m128i d = _mm_loadl_epi64((const __m128i *)dst);
      _mm_storel_epi64((__m128i *)dst, _mm_avg_epu8(d, t));
    } else {
      int32_t d32;
      memcpy(&d32, dst, 4);
      d32 = _mm_cvtsi128_si32(_mm_avg_epu8(_mm_cvtsi32_si128(d32), t));
      memcpy(dst, &d32, 4);
    }
  }
}

/* An averaging helper: `filter_fn` into a scratch block, 64 rows at a time
   (a VP9 block is at most 64 high), then averaged into dst. */
static void filter_then_average(filter8_1dfunction *filter_fn, const uint8_t *src, ptrdiff_t src_stride,
                                uint8_t *dst, ptrdiff_t dst_stride, uint32_t h, const int16_t *filter, int w) {
  DECLARE_ALIGNED(16, uint8_t, tmp[16 * 64]);
  while (h) {
    const uint32_t n = h < 64 ? h : 64;
    filter_fn(src, src_stride, tmp, 16, n, filter);
    average_into(dst, dst_stride, tmp, w, n);
    src += src_stride * n;
    dst += dst_stride * n;
    h -= n;
  }
}

#define SHIM_ARGS                                                                       \
  const uint8_t *src, ptrdiff_t src_stride, uint8_t *dst, ptrdiff_t dst_stride, uint32_t h, \
      const int16_t *filter
#define SHIM_PASS src, src_stride, dst, dst_stride, h, filter

/* With the `shim-stats` feature, calls per group, so a test can show which
   paths a clip reaches: 0 SSSE3 8-tap, 1 SSSE3 8-tap averaging, 2 bilinear
   (either ISA, either kind), 3 SSE2 8-tap (either kind). */
#if defined(SK_VPX_SHIM_STATS)
#include <intrin.h>
static volatile long long shim_calls[4];
#define COUNT(group) _InterlockedIncrement64(&shim_calls[group])
void sk_vpx_shim_stats(long long out[4]) {
  for (int i = 0; i < 4; i++) out[i] = shim_calls[i];
}
#else
#define COUNT(group) ((void)0)
#endif

/* SSSE3, 8-tap: the paths an SSSE3 or AVX2 CPU takes (AVX2 hands its 4- and
   8-wide averaging work to these). */
void vpx_filter_block1d16_h8_ssse3(SHIM_ARGS) {
  COUNT(0);
  h8_16_ssse3(SHIM_PASS);
}
void vpx_filter_block1d16_v8_ssse3(SHIM_ARGS) {
  COUNT(0);
  v8_16_ssse3(SHIM_PASS);
}
void vpx_filter_block1d4_v8_ssse3(SHIM_ARGS) {
  COUNT(0);
  convolve_c(SHIM_PASS, 4, 1, 8, 0);
}
void vpx_filter_block1d16_h8_avg_ssse3(SHIM_ARGS) {
  COUNT(1);
  filter_then_average(h8_16_ssse3, SHIM_PASS, 16);
}
void vpx_filter_block1d16_v8_avg_ssse3(SHIM_ARGS) {
  COUNT(1);
  filter_then_average(v8_16_ssse3, SHIM_PASS, 16);
}
void vpx_filter_block1d8_h8_avg_ssse3(SHIM_ARGS) {
  COUNT(1);
  filter_then_average(vpx_filter_block1d8_h8_intrin_ssse3, SHIM_PASS, 8);
}
void vpx_filter_block1d8_v8_avg_ssse3(SHIM_ARGS) {
  COUNT(1);
  filter_then_average(vpx_filter_block1d8_v8_intrin_ssse3, SHIM_PASS, 8);
}
void vpx_filter_block1d4_h8_avg_ssse3(SHIM_ARGS) {
  COUNT(1);
  filter_then_average(vpx_filter_block1d4_h8_intrin_ssse3, SHIM_PASS, 4);
}
void vpx_filter_block1d4_v8_avg_ssse3(SHIM_ARGS) {
  COUNT(1);
  convolve_c(SHIM_PASS, 4, 1, 8, 1);
}

/* 2-tap (bilinear kernels only; libvpx's encoder does not pick them). */
#define BILINEAR(w, dir, vertical, isa)                    \
  void vpx_filter_block1d##w##_##dir##2_##isa(SHIM_ARGS) {     \
    COUNT(2);                                                  \
    convolve_c(SHIM_PASS, w, vertical, 2, 0);                  \
  }                                                            \
  void vpx_filter_block1d##w##_##dir##2_avg_##isa(SHIM_ARGS) { \
    COUNT(2);                                                  \
    convolve_c(SHIM_PASS, w, vertical, 2, 1);                  \
  }
BILINEAR(16, h, 0, ssse3)
BILINEAR(16, v, 1, ssse3)
BILINEAR(8, h, 0, ssse3)
BILINEAR(8, v, 1, ssse3)
BILINEAR(4, h, 0, ssse3)
BILINEAR(4, v, 1, ssse3)
BILINEAR(16, h, 0, sse2)
BILINEAR(16, v, 1, sse2)
BILINEAR(8, h, 0, sse2)
BILINEAR(8, v, 1, sse2)
BILINEAR(4, h, 0, sse2)
BILINEAR(4, v, 1, sse2)

/* SSE2, 8-tap: only a CPU without SSSE3 dispatches to the SSE2 convolutions. */
#define EIGHT_TAP_SSE2(w, dir, vertical)                  \
  void vpx_filter_block1d##w##_##dir##8_sse2(SHIM_ARGS) {     \
    COUNT(3);                                                 \
    convolve_c(SHIM_PASS, w, vertical, 8, 0);                 \
  }                                                           \
  void vpx_filter_block1d##w##_##dir##8_avg_sse2(SHIM_ARGS) { \
    COUNT(3);                                                 \
    convolve_c(SHIM_PASS, w, vertical, 8, 1);                 \
  }
EIGHT_TAP_SSE2(16, h, 0)
EIGHT_TAP_SSE2(16, v, 1)
EIGHT_TAP_SSE2(8, h, 0)
EIGHT_TAP_SSE2(8, v, 1)
EIGHT_TAP_SSE2(4, h, 0)
EIGHT_TAP_SSE2(4, v, 1)
