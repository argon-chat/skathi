#!/usr/bin/env bash
#
# Re-vendors the ThorVG C++ subset under `thorvg/` from the pinned upstream
# release. Run it from anywhere; it rewrites `crates/vn-thorvg-sys/thorvg/`
# in place and is idempotent.
#
#     bash crates/vn-thorvg-sys/tools/vendor-thorvg.sh
#
# The tree it produces is PRISTINE upstream — no patches. That is a deliberate
# property: `git diff` against a fresh run of this script is the audit, and a
# version bump is "edit VERSION/SHA256, re-run, commit". See VENDOR.md.
set -euo pipefail

VERSION="1.1.0"
SHA256="73943bcf918c9ffdf11acc53052ba5cb60a7047b1cfb2067afeb62e0c4d9417d"
URL="https://github.com/thorvg/thorvg/archive/refs/tags/v${VERSION}.tar.gz"

crate_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="${crate_dir}/thorvg"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

echo "fetching ${URL}"
curl -fsSL -o "${work}/thorvg.tar.gz" "${URL}"

actual="$(sha256sum "${work}/thorvg.tar.gz" | cut -d' ' -f1)"
if [ "${actual}" != "${SHA256}" ]; then
    echo "SHA256 MISMATCH for ${URL}" >&2
    echo "  expected ${SHA256}" >&2
    echo "  actual   ${actual}" >&2
    exit 1
fi
echo "sha256 ok: ${actual}"

tar -xzf "${work}/thorvg.tar.gz" -C "${work}"
src="${work}/thorvg-${VERSION}"

rm -rf "${dest}"
mkdir -p "${dest}"

# Copy one upstream directory's *.cpp / *.h (non-recursive unless -r).
copy_dir() {
    local recursive="" relative="$1"
    if [ "$1" = "-r" ]; then recursive="yes"; relative="$2"; fi
    local from="${src}/${relative}" to="${dest}/${relative}"
    mkdir -p "${to}"
    if [ -n "${recursive}" ]; then
        (cd "${from}" && find . \( -name '*.cpp' -o -name '*.h' -o -name 'LICENSE' \) -print0 \
            | while IFS= read -r -d '' f; do
                mkdir -p "${to}/$(dirname "${f}")"
                cp "${from}/${f}" "${to}/${f}"
              done)
    else
        (cd "${from}" && find . -maxdepth 1 \( -name '*.cpp' -o -name '*.h' -o -name 'LICENSE' \) \
            -exec cp {} "${to}/" \;)
    fi
}

cp "${src}/LICENSE" "${dest}/LICENSE"
mkdir -p "${dest}/inc" && cp "${src}/inc/thorvg.h" "${dest}/inc/thorvg.h"

# Engine. `gpu_engine/` is deliberately absent: we only build the software
# rasteriser, so the GL/WebGPU backends are dead weight and would drag in
# system GL headers, breaking the no-system-deps guarantee.
copy_dir src/common
copy_dir src/renderer
copy_dir src/renderer/cpu_engine
copy_dir src/bindings/capi

# Loaders. `raw` is what Picture::load(uint32_t*) uses and the C API always
# references it; the rest match `rasterlottie`'s feature set so the two
# backends are compared at equal capability.
copy_dir src/loaders/raw
copy_dir src/loaders/lottie
copy_dir -r src/loaders/lottie/rapidjson
copy_dir -r src/loaders/lottie/jerryscript
copy_dir src/loaders/png
copy_dir src/loaders/jpg
copy_dir src/loaders/sfnt

files=$(find "${dest}" -type f | wc -l)
bytes=$(du -sk "${dest}" | cut -f1)
echo "vendored ThorVG ${VERSION}: ${files} files, ${bytes} KiB -> ${dest}"
