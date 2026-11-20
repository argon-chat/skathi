#!/usr/bin/env bash
set -euo pipefail

scratch="$RUNNER_TEMP/skathi-macos26-inputs"
results="$RUNNER_TEMP/skathi-macos26-results"
mkdir -p "$scratch" "$results"
target=aarch64-apple-darwin
profile="target/$target/release"
features=prod,storages,clipboard,no-devtools
measurements="${MEASUREMENTS:-all}"
[[ "$measurements" == all || "$measurements" == surface || "$measurements" == controls ]]

{
  sw_vers
  uname -m
  printf 'Console user: '
  /usr/bin/stat -f '%Su' /dev/console
  printf 'Runner user: '
  id -un
  sysctl hw.model hw.memsize hw.ncpu
  xcodebuild -version
  rustc --version
  system_profiler SPDisplaysDataType -detailLevel mini
  df -h "$RUNNER_TEMP"
} | tee "$results/machine.txt"

xcrun clang -O2 -fobjc-arc .github/scripts/macos26-measure.m -framework AppKit -framework CoreGraphics -o "$scratch/measure"
"$scratch/measure" displays | tee "$results/displays.txt"

app="$scratch/app"
mkdir -p "$app/src"
ln -s "$PWD/node_modules" "$app/node_modules"
cat > "$app/package.json" <<'JSON'
{"name":"skathi-macos26-diagnostics","private":true,"type":"module","dependencies":{"@skathi-native/runtime":"0.7.0"}}
JSON
publish() {
  bun packages/cli/cli/sk.ts publish "$app" --payload-only --no-aot --out "$1"
  test -s "$1/.sk/app.skpk"
}
printf 'run\tmax_kernel_footprint_bytes\tmedian_11_to_13_second_bytes\tfirst_frame\n' > "$results/summary.tsv"

measure() (
  local name=$1 host=$2 payload=$3
  shift 3
  local dir="$results/$name"
  mkdir -p "$dir" "$scratch/data/$name"
  local pid= sampler= probe1= probe2=
  cleanup() {
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    for child in "$sampler" "$probe1" "$probe2"; do
      if [[ -n "$child" ]]; then kill "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true; fi
    done
  }
  trap cleanup EXIT
  if [[ "$name" == surface-* ]]; then
    export SK_LOG=gpui_macos::surface_pacing=trace
  fi
  printf '%s: %s\n' "$name" "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" | tee "$dir/launched.txt"
  SK_PAYLOAD="$payload" SK_DATA_DIR="$scratch/data/$name" SK_WINDOW_FOCUS=raise SK_PERF=1 "$host" "$@" > "$dir/host.log" 2>&1 &
  pid=$!
  "$scratch/measure" sample "$pid" > "$dir/footprint-samples.tsv" 2>"$dir/sampler-errors.txt" &
  sampler=$!
  (
    sleep 0.4
    vmmap -summary "$pid" > "$dir/vmmap-0.4s.txt" 2>&1 || true
  ) &
  probe1=$!
  (
    sleep 2
    "$scratch/measure" window "$pid" > "$dir/windows-2s.txt" 2>&1 || true
    footprint -v "$pid" > "$dir/footprint-2s.txt" 2>&1 || true
  ) &
  probe2=$!
  sleep 12
  "$scratch/measure" window "$pid" > "$dir/windows-12s.txt" 2>&1 || true
  footprint -v "$pid" > "$dir/footprint-12s.txt" 2>&1 || true
  vmmap -summary "$pid" > "$dir/vmmap-12s.txt" 2>&1 || true
  wait "$sampler" || true
  wait "$probe1" || true
  wait "$probe2" || true

  local first_frame peak median flags=0
  first_frame=$(sed -n 's/^\[sk-perf\] boot->first-frame //p' "$dir/host.log" | sed -n '1p')
  if [[ "$name" == control-* ]]; then
    first_frame=$(awk '
      /^draw / && !submitted {submitted=$3}
      /^presented / && $3 > 0 && !shown {shown=$3}
      END {
        if (shown) printf "%.1fms (presented)", shown
        else if (submitted) printf "%.1fms (submitted)", submitted
      }
    ' "$dir/host.log")
  fi
  peak=$(awk -F '\t' 'NR > 1 && $3 > m {m=$3} END {print m+0}' "$dir/footprint-samples.tsv")
  median=$(awk -F '\t' '$1 >= 11000 && $1 <= 13000 {print $2}' "$dir/footprint-samples.tsv" |
    sort -n | awk '{value[NR]=$1} END {if (NR) print value[int((NR+1)/2)]}')
  printf '%s\t%s\t%s\t%s\n' "$name" "$peak" "$median" "$first_frame" | tee -a "$results/summary.tsv"
  for elapsed in 2 12; do
    awk -v elapsed="$elapsed" '
      /CAMetalLayer Display Drawable/ && match($0, /SID: 0x[[:xdigit:]]+/) {
        ids[substr($0, RSTART+5, RLENGTH-5)]=1
      }
      END {
        for (id in ids) {count++; found=found " " id}
        printf "%d distinct drawable IOSurfaces at %ds:%s\n", count, elapsed, found
      }
    ' "$dir/footprint-${elapsed}s.txt"
  done
  if [[ -z "$first_frame" || -z "$median" || "$peak" == 0 ]] ||
    ! grep -q '^on-screen window ' "$dir/windows-12s.txt"; then
    echo "::warning::$name did not produce a verified on-screen frame and complete footprint samples"
    sed -n '1,100p' "$dir/host.log"
    flags=1
  fi
  if [[ "$name" == surface-* ]] &&
      ! grep -Eq '\[surface-diagnostics\] playing .+"shown":[1-9]' "$dir/host.log"; then
    echo "::warning::$name left no positive presentation-count marker"
    flags=1
  fi
  return "$flags"
)

status=0
if [[ "$measurements" == all ]]; then
  cargo build --locked --release -p sk-app --bin sk-app --no-default-features --features "$features" --target "$target"
  cp "$profile/sk-app" "$scratch/sk-app-precompiled"
  cargo build --locked --release -p sk-app --bin sk-app --no-default-features --features "$features,runtime-shaders" --target "$target"
  cp "$profile/sk-app" "$scratch/sk-app-runtime"
  cat > "$app/sk.config.json" <<'JSON'
{"name":"skathi-macos26-diagnostics","window":{"title":"Skathi diagnostics","width":600,"height":400}}
JSON
  cat > "$app/src/main.ts" <<'TS'
import { createApp, h } from "@skathi-native/runtime";
createApp({
  render: () => h("div", { style: "position:absolute; inset:0; background:#245784" }),
}).mount();
TS
  publish "$scratch/minimal"
  for run in 1 2 3; do
    if [[ "$run" == 2 ]]; then
      modes=(runtime precompiled)
    else
      modes=(precompiled runtime)
    fi
    for mode in "${modes[@]}"; do
      measure "$mode-$run" "$scratch/sk-app-$mode" "$scratch/minimal/.sk/app.skpk" || status=1
    done
  done
fi

xcrun clang -O2 -fobjc-arc .github/scripts/macos26-drawables.m -framework AppKit -framework CoreGraphics \
  -framework QuartzCore -framework Metal -framework IOSurface -o "$scratch/drawables"
for run in 1 2; do
  for test in 'next 3 2' 'link 3 2' 'link 3 1' 'link 2 2' 'link 2 1' 'hybrid 3 2' 'idle 3 2'; do
    read -r mode count latency <<< "$test"
    measure "control-$mode-$count-$latency-$run" "$scratch/drawables" "" "$mode" "$count" "$latency" || status=1
  done
done

if [[ "$measurements" == controls ]]; then exit "$status"; fi

cargo rustc --locked -p sk-app --example surface --no-default-features --features "$features" --target "$target" \
  --config "profile.dev.package.'*'.debug=0" -- \
  -C link-arg=-Wl,-ObjC -C link-arg=-Wl,-rpath,/usr/lib/swift \
  -C link-arg=-Wl,-no_deduplicate -C link-arg=-Wl,-w \
  2>&1 | tee "$results/surface-build.log"
cat > "$app/src/main.ts" <<'TS'
import { appHost, createApp, h } from "@skathi-native/runtime";
createApp({
  render: () =>
    h("div", { style: "position:absolute; inset:0; background:#111" }, [
      h("sk-surface", {
        name: "video",
        style: "position:absolute; inset:0; object-fit:fill; dynamic-range-limit:standard;",
      }),
      h("sk-surface", {
        name: "overlay",
        style: "position:absolute; inset:65% 0 0; object-fit:fill; dynamic-range-limit:standard;",
      }),
    ]),
}).mount();
setTimeout(async () => {
  try {
    let frame = 0;
    const start = performance.now();
    const nextFrame = async () => {
      if (performance.now() - start >= 14000) return;
      const color = (++frame * 19) % 256;
      try {
        await appHost.call("present", "video", "iosurface", [color, 40, 255 - color, 255], 1920, 1108);
      } catch (error) {
        console.error("[surface-diagnostics] video frame failed", error);
      }
      setTimeout(nextFrame, Math.max(0, start + frame * (1000 / 24) - performance.now()));
    };
    void nextFrame();
    await appHost.call("present", "overlay", "metal", [24, 90, 180, 255], 3456, 1994);
    await appHost.call("present", "overlay", "metal", [50, 90, 180, 255], 3456, 1994);
    setTimeout(async () => {
      console.log("[surface-diagnostics] playing video", JSON.stringify(await appHost.call("stats", "video")));
    }, 1500);
    setTimeout(async () => {
      await appHost.call("present", "overlay", "metal", [90, 90, 180, 255], 3456, 300);
      console.log("[surface-diagnostics] resized overlay");
    }, 3500);
    setTimeout(async () => {
      await appHost.call("clear", "overlay");
      console.log("[surface-diagnostics] cleared overlay", JSON.stringify(await appHost.call("stats", "overlay")));
    }, 6500);
  } catch (error) {
    console.error("[surface-diagnostics] failed", error);
  }
}, 800);
TS
for hybrid in false true; do
  printf '{"name":"skathi-macos26-diagnostics","window":{"title":"Skathi surface diagnostics","width":960,"height":540,"lowLatencySurface":%s}}\n' \
    "$hybrid" > "$app/sk.config.json"
  publish "$scratch/surface-$hybrid"
  measure "surface-$hybrid" "target/$target/debug/examples/surface" "$scratch/surface-$hybrid/.sk/app.skpk" || status=1
done
cat "$results/summary.tsv"
exit "$status"
