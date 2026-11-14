#!/usr/bin/env bash
#
# Fetches the real-world Lottie corpus that `examples/compare.rs` measures the backends against,
# into `tests/corpus/` (gitignored).
#
#     bash crates/sk-lottie/tools/fetch-corpus.sh
#
# Not committed: these are third-party After Effects exports bundled with Samsung's rlottie, with
# no per-file licence statement, fine to measure against and wrong to redistribute, so the corpus
# is pinned by commit and fetched.
set -euo pipefail

# Samsung/rlottie @ master, 2026-08-19, pinned.
REPO="Samsung/rlottie"
COMMIT="8de0d9e6ca80ffef654965505981727b9fa06a51"
BASE="https://raw.githubusercontent.com/${REPO}/${COMMIT}/example/resource"

# Two groups, reported separately: `real-world` (ordinary exports a `<lottie>` will be pointed at;
# `bell.json` is the one file the `normalize` pass cannot rescue for `rasterlottie`) and `probes`
# (each carries a feature `rasterlottie` does not support: text layers, layer effects,
# expressions, time remapping, embedded images).
REAL_WORLD=(
    bell.json
    done.json
    a_mountain.json
    emoji_shock.json
    cooking.json
    1643-exploding-star.json
    party_penguin.json
    fingerprint_success.json
)
PROBES=(
    worm.json                  # text + expressions + effects + time remap
    mnemonics.json             # text + expressions + effects + mask
    windmill.json              # expressions + time remap, and little else
    leap_frog_loader.json      # layer effects + expressions + text
    marker.json                # time remapping
    image_embedded.json        # base64 image asset
    5344-honey-sack-hud.json   # 138 KB of masks and trim paths
    jolly_walker.json          # expressions + solid layers
)

dest="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/tests/corpus"
mkdir -p "${dest}/real-world" "${dest}/probes"

fetch() {
    local group="$1" name="$2"
    local out="${dest}/${group}/${name}"
    if [ -f "${out}" ]; then
        echo "  have ${group}/${name}"
        return
    fi
    # The names contain apostrophes and spaces upstream; curl handles the ones selected here as-is.
    curl -fsSL -o "${out}" "${BASE}/${name}"
    echo "  got  ${group}/${name} ($(wc -c < "${out}") bytes)"
}

echo "fetching corpus from ${REPO}@${COMMIT:0:8}"
for name in "${REAL_WORLD[@]}"; do fetch real-world "${name}"; done
for name in "${PROBES[@]}"; do fetch probes "${name}"; done
echo "corpus in ${dest}"
