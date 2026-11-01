/*
 * vn-vpx-sys: libvpx's allocator (vpx_mem/vpx_mem.c, not vendored), with a
 * per-decoder account.
 *
 * Every allocation libvpx's decoder makes goes through these four functions
 * (vpx_image.c's one `calloc` is for a vpx_image_t the decoder never asks
 * for). The arithmetic and the overflow guard are vpx_mem.c's; what is added
 * is a small header before each block naming the account that paid for it, so
 * src/lib.rs can say how many bytes a decoder holds, exactly, and refuse an
 * allocation past a cap. A refused allocation is libvpx's ordinary out-of-
 * memory path (VPX_CODEC_MEM_ERROR), which it handles without leaking.
 *
 * The account is chosen by the calling thread: src/lib.rs brackets every
 * libvpx call with vn_vpx_account_enter/leave. Libvpx runs with
 * CONFIG_MULTITHREAD 0, so every allocation happens on that thread, inside
 * that call; a free uses the account recorded in the block, so it is right
 * whichever call releases it.
 */
#include <limits.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "vpx_mem/vpx_mem.h"

#if defined(_MSC_VER)
#define VN_THREAD_LOCAL __declspec(thread)
#else
#define VN_THREAD_LOCAL _Thread_local
#endif

/* Mirrored by `Account` in src/lib.rs. */
typedef struct vn_vpx_account {
  size_t live;  /* bytes held now, headers and alignment slack included */
  size_t peak;  /* the most `live` has been */
  size_t limit; /* 0: no cap */
} vn_vpx_account;

static VN_THREAD_LOCAL vn_vpx_account *current_account;

void vn_vpx_account_enter(vn_vpx_account *account) { current_account = account; }
void vn_vpx_account_leave(void) { current_account = NULL; }

/* vpx_mem.c's limit: an allocation request above it is a corrupt size. */
#if SIZE_MAX > (1ULL << 40)
#define VN_MAX_ALLOCABLE_MEMORY (1ULL << 40)
#else
#define VN_MAX_ALLOCABLE_MEMORY ((1ULL << 31) - (1 << 16))
#endif

/* Before each block: the address malloc returned, the account, the bytes
   charged. */
#define HEADER_WORDS 3
#define HEADER_BYTES (HEADER_WORDS * sizeof(size_t))
#define DEFAULT_ALIGNMENT (2 * sizeof(void *))

static int size_overflows(uint64_t nmemb, uint64_t size) {
  if (nmemb == 0) return 0;
  if (size > VN_MAX_ALLOCABLE_MEMORY / nmemb) return 1;
  return (uint64_t)(size_t)(nmemb * size) != nmemb * size;
}

void *vpx_memalign(size_t align, size_t size) {
  vn_vpx_account *const account = current_account;
  uint64_t total;
  unsigned char *base;
  uintptr_t block;
  size_t *header;
  if (align < sizeof(size_t)) align = sizeof(size_t);
  total = (uint64_t)size + align - 1 + HEADER_BYTES;
  if (size_overflows(1, total)) return NULL;
  if (account && account->limit && (account->live > account->limit || total > account->limit - account->live))
    return NULL;
  base = (unsigned char *)malloc((size_t)total);
  if (!base) return NULL;
  block = ((uintptr_t)base + HEADER_BYTES + align - 1) & ~(uintptr_t)(align - 1);
  header = (size_t *)block - HEADER_WORDS;
  header[0] = (size_t)base;
  header[1] = (size_t)account;
  header[2] = (size_t)total;
  if (account) {
    account->live += (size_t)total;
    if (account->live > account->peak) account->peak = account->live;
  }
  return (void *)block;
}

void *vpx_malloc(size_t size) { return vpx_memalign(DEFAULT_ALIGNMENT, size); }

void *vpx_calloc(size_t num, size_t size) {
  void *block;
  if (size_overflows(num, size)) return NULL;
  block = vpx_malloc(num * size);
  if (block) memset(block, 0, num * size);
  return block;
}

void vpx_free(void *memblk) {
  size_t *header;
  vn_vpx_account *account;
  if (!memblk) return;
  header = (size_t *)memblk - HEADER_WORDS;
  account = (vn_vpx_account *)header[1];
  if (account) account->live -= header[2];
  free((void *)header[0]);
}
