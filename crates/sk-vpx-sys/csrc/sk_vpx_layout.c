/*
 * sk-vpx-sys: the sizes and offsets src/lib.rs mirrors by hand, so a test can
 * compare the Rust declarations with the C compiler's layout.
 */
#include <stddef.h>

#include "vpx/vpx_decoder.h"

size_t sk_vpx_layout(int which) {
  switch (which) {
    case 0: return sizeof(vpx_codec_ctx_t);
    case 1: return offsetof(vpx_codec_ctx_t, err);
    case 2: return offsetof(vpx_codec_ctx_t, priv);
    case 3: return sizeof(vpx_codec_dec_cfg_t);
    case 4: return sizeof(vpx_codec_stream_info_t);
    case 5: return sizeof(vpx_image_t);
    case 6: return offsetof(vpx_image_t, d_w);
    case 7: return offsetof(vpx_image_t, x_chroma_shift);
    case 8: return offsetof(vpx_image_t, planes);
    case 9: return offsetof(vpx_image_t, stride);
    case 10: return offsetof(vpx_image_t, fb_priv);
    case 11: return VPX_DECODER_ABI_VERSION;
    default: return (size_t)-1;
  }
}
