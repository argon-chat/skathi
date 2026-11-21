---
title: Shipping an embedded app
label: Shipping
description: Where the payload lives (beside the exe or inside it), the profile and storage encryption, binary size with opt-level and rust-lld, and running on macOS, where AppKit owns the main thread.
section: Embedding
order: 9
---

Your exe is yours to package, sign and update. What Skathi needs from you at release time is short: the payload, a place for the profile, and on macOS the main thread.

## The payload

<div class="tradeoffs">
<div class="tradeoff" data-tone="gain">

**Beside the exe: `payload_file`**

- The app updates without rebuilding Rust: replace `app.skpk` (atomically, never in place) and restart.
- Mapped read-only for the app's lifetime, so a large payload costs no copy.
- Keep asset packs (`fonts.skpk`, `assets.skpk`, `icu.skpk`) with the payload. AOT also needs the matching image library. See [AOT for your own executable](#aot-for-your-own-executable-since-07).

</div>
<div class="tradeoff" data-tone="gain">

**Inside the exe: `payload_static`**

- The executable carries the payload in its signature.
- `payload_static(include_bytes!("../app/dist/release/.sk/app.skpk"))` borrows the bytes with no copy.
- Every app change means a Rust rebuild.
- AOT uses a separate image library for the embedded payload.

</div>
</div>

```rust
// Beside the exe, wherever the installer put it:
let payload = std::env::current_exe()?.with_file_name("app.skpk");
let builder = skathi::Builder::new().payload_file(payload);

// Inside it:
static APP: &[u8] = include_bytes!("../app/dist/release/.sk/app.skpk");
let builder = skathi::Builder::new().payload_static(APP);
```

`payload_bytes(vec)` takes owned bytes, for a payload you download, decrypt or unpack yourself. With none of the three, a `prod` build reads `SK_PAYLOAD`, and fails with `Error::NoPayload` when that is empty too.

## AOT for your own executable (since 0.7)

Use a [Skathi source checkout](https://github.com/argon-chat/skathi) with its submodules initialized, at the same version as the CLI and your Rust dependency. Build your embedder with `prod` and `native-aot`.

### Windows x64

From the Skathi checkout, prepare the compiler and its pinned JavaScriptCore builtin table and build the empty image container.

```powershell
bun install
pwsh -File vendor/redjs/setup.ps1
bun tools/sk-aot/setup.ts
Push-Location tools/sk-aot
cargo build --release --locked
Pop-Location
cargo build --release --manifest-path crates/sk-image/Cargo.toml --target x86_64-pc-windows-msvc
$env:REDJS_AOT_BUILTIN_TABLE_DIR = (Resolve-Path tools/sk-aot/.cache/table).Path
$skAot = (Resolve-Path tools/sk-aot/target/x86_64-pc-windows-msvc/release/sk-aot.exe).Path
$skImage = (Resolve-Path crates/sk-image/target/x86_64-pc-windows-msvc/release/skimage.dll).Path
```

In your own executable's Cargo manifest, enable the loader along with the features your app uses:

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.7", default-features = false, features = ["prod", "native-aot", "no-devtools"] }
```

Build the exe with that environment and use the embedder's [static CRT configuration](/embedding/first-embedder/#3-the-static-crt-windows) and [build script](/embedding/first-embedder/#4-the-build-script) to apply Skathi's Windows linker arguments and resources. Put the built `skimage.dll` next to it for packaging. Then compile the payload for that exact exe:

```powershell
cargo build --release --target x86_64-pc-windows-msvc
$release = '.\target\x86_64-pc-windows-msvc\release'
Copy-Item $skImage $release
sk publish .\app --aot --payload-only --workload "$release\my-app.exe" --aot-compiler $skAot
```

This writes `app.skpk`, `app.image.dll` and any asset or ICU packs to `app/dist/release/.sk`. The manifest `sk/app.native.json` is inside `app.skpk`. It binds the image to the bundle, the host's AOT ABI and JavaScriptCore pin, the builtin table, and the host preludes. The CLI queries your exe with `--sk-host-info` before compiling. `skathi::Builder` answers that flag before loading a payload. Keep the image and payload from the same build, and rebuild both when you upgrade Skathi.

Ship `app.image.dll` and the packs beside `app.skpk` when using `Builder::payload_file`, or beside your exe when using `Builder::payload_static`. `skimage.dll` is a build input. If you use `payload_static`, build a host with `payload_file` first to produce the payload, then rebuild with `include_bytes!`.

### macOS arm64

On an Apple Silicon Mac, prepare the compiler, table and Mach-O template in the Skathi checkout.

```bash
bun install
bash vendor/redjs/setup.sh
bun tools/sk-aot/setup.ts
(cd tools/sk-aot && cargo build --release --locked --target aarch64-apple-darwin)
cargo build --release --manifest-path crates/sk-image/Cargo.toml --target aarch64-apple-darwin
export REDJS_AOT_BUILTIN_TABLE_DIR="$PWD/tools/sk-aot/.cache/table"
skAot="$PWD/tools/sk-aot/target/aarch64-apple-darwin/release/sk-aot"
skImage="$PWD/crates/sk-image/target/aarch64-apple-darwin/release/libskimage.dylib"
```

Use the same `skathi` Cargo features as above and apply Skathi's macOS [linker arguments](/embedding/first-embedder/#4-the-build-script). Call `run_on_main_thread` from your embedder. From your embedder's project directory, build the executable and use it to compile the payload.

```bash
cargo build --release --target aarch64-apple-darwin
release=target/aarch64-apple-darwin/release
cp "$skImage" "$release/"
sk publish ./app --aot --payload-only --workload "$release/my-app" --aot-compiler "$skAot"
```

This writes `app.skpk`, `app.image.dylib` and any packs to `app/dist/release/.sk`. For a bare executable with `payload_file`, keep the image library and packs beside `app.skpk`. For a `.app` using `payload_file`, put the payload and its packs in `Contents/Resources` and pass that payload path to `Builder::payload_file`. Both `payload_file` and `payload_static` look for `app.image.dylib` in `Contents/Frameworks` in this layout. `libskimage.dylib` is a build input. Sign the image library before signing the `.app`. See the [macOS bundle](/guides/aot/#macos-bundle-since-07) for signing and notarization.

### Leave out the JIT and WebAssembly

From the same Skathi checkout, generate the trimmed archive and set `REDJS_NO_JIT_DIR` before building the embedder:

```powershell
bun scripts/aot/no-jit.ts --out .\target\aot-no-jit --remove JIT,WebAssembly
$env:REDJS_NO_JIT_DIR = (Resolve-Path .\target\aot-no-jit).Path
```

On Apple Silicon, run the same script with the macOS prebuilt from `vendor/redjs/setup.sh`.

```bash
bun scripts/aot/no-jit.ts --out target/aot-no-jit --remove JIT,WebAssembly
export REDJS_NO_JIT_DIR="$PWD/target/aot-no-jit"
```

Add `no-wasm` to your `skathi` features for that archive. It implies `no-jit` and `native-aot`. For a host that keeps WebAssembly, use `no-jit` with an archive generated using `--remove JIT`. Both hosts require an AOT payload and retain the interpreter. The experimental `no-interpreter` feature is for engine qualification and is not exposed by `skathi`.

### Include the inspector

Use `native-inspector` in place of `no-devtools`. In the Skathi checkout, run `bun scripts/aot/prepare-inspector.ts --out target/inspector --cache target/aot-cache --table tools/sk-aot/.cache/table --container <template>`, using `skimage.dll` on Windows or `libskimage.dylib` on macOS. Set `SK_DEVTOOLS_BUNDLE` and `SK_DEVTOOLS_NATIVE_MANIFEST` to the `bundle` and `manifest` paths in `target/inspector/build.json`. Set `SK_DEVTOOLS_NATIVE_IMAGE_CONTAINER` to `inspector.image.dll` or `inspector.image.dylib` before rebuilding.

Keep the inspector library beside the build executable for `sk publish --payload-only` and set `"publishDevtools": true` in the app configuration. When you package a `.app`, put the inspector library in `Contents/Frameworks` and sign it with the application image. A bare executable loads the inspector library from beside itself. To open devtools with F12, point `Builder::inspector_exe` to the executable and delegate its leading `--devtools` flag to `skathi::run_exe()`.

## Startup graphics memory on macOS

On an Apple M4 with macOS 26.4, a headless Metal probe loaded the same renderer shaders from source and from a precompiled library, then created the seven render pipelines. Footprint at that point was about 5–6 MiB in either case. The first clear and present of a 3456 × 2234 BGRA8 drawable raised both builds to about 97–98 MiB. `vmmap -summary` reported about 49 MiB resident as `owned unmapped (graphics)`. About 48 MiB of the footprint dropped after the frame settled.

In a separate windowed test on macOS 26.7.2, two release hosts ran the same minimal payload in a 600 × 400 window on a virtual 60 Hz SDR display. Across three launches each, the median peak process footprint was 38.2 MiB with precompiled shaders and 38.1 MiB with runtime shaders. These runs did not reproduce the 229 MiB whole-app peak reported on the M3 Max. Leaving `runtime-shaders` off moves library compilation to the build. The first frame still allocates GPU resources.

On some Macs the first present to a `CAMetalLayer` costs about 190 MiB of graphics memory for about a second, in any process: an M3 Max with its built-in XDR display showed it in a 40-line AppKit program with no Skathi in it. The system allocates it, `MTLDevice.currentAllocatedSize` does not count it, and the footprint settles afterwards, so a `footprintPeakMB` that includes it is not the app's cost.

## The native modules' types (since 0.4)

A release exe answers `--sk-host-types` like a debug one: it prints the TypeScript of its [native modules](/embedding/native-modules/) and exits, which is how `sk types` reads them. Commit the generated `src/native-modules.d.ts` with the app, so the payload builds on a machine without your Rust build. Anyone can read those declarations out of your exe, as they could read the names in its strings: leave secrets out of module and type names.

## The profile

localStorage, cookies, IndexedDB, secrets and the HTTP and bytecode caches live in one profile folder per app.

| On `Builder`              | Effect                                                                                                                                                                      |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `data_dir(path)`          | Where the profile lives (absolute). The caches go in its `cache/http` and `cache/jsc`. `SK_DATA_DIR` overrides it                                                           |
| `storage_encrypted(true)` | Seal storage with the OS provider (DPAPI, the Keychain, the Secret Service); existing plain-text localStorage is sealed before boot. Needs a provider: there is no fallback |
| `prepare_data_dir(f)`     | Runs before storage loads, after single-instance handling: migrate an old profile, move it, check free space. An error stops startup                                        |

`skathi::encrypt_local_storage_at(dir)` seals a profile that is not in use, for an installer or an upgrade step.

Without `data_dir`, storage goes where Skathi's own exe puts it, under the app's name (on Windows `%LOCALAPPDATA%\<name>\storage`). Set it when your program already has a data folder, so the user finds one, not two.

## Binary size

Cargo profiles come from your workspace, not from Skathi's, so your exe is built the way your `Cargo.toml` says. Skathi's own release hosts use two settings that take about 12 MB off on Windows without a measurable slowdown, and your exe can use them too:

```toml title="Cargo.toml"
[profile.release]
opt-level = "s"
lto = "thin"
codegen-units = 1

# Layout, paint and the renderer stay at full speed: at "s" they take 15-30% longer per frame.
[profile.release.package]
your-crate.opt-level = 3 # under LTO the final crate's level applies to every crate's code
sk-app.opt-level = 3
sk-runtime.opt-level = 3
sk-tree.opt-level = 3
sk-css.opt-level = 3
sk-elements.opt-level = 3
sk-effects.opt-level = 3
skathi-gpui.opt-level = 3
skathi-gpui-windows.opt-level = 3
skathi-taffy.opt-level = 3
```

```toml title=".cargo/config.toml"
[target.x86_64-pc-windows-msvc]
linker = "rust-lld"
rustflags = ["-C", "target-feature=+crt-static"]
```

With `rust-lld`, which ships with rustc, the crate's build script tells yours to fold identical JavaScriptCore functions the safe way (`/OPT:SAFEICF`), about 5 MB more. With `link.exe` it turns folding off, because folding there breaks `BigInt` and `Symbol`. The full override list, image and audio decoders included, is in the `[profile.release.package]` section of Skathi's own `Cargo.toml`.

More levers, measured on a minimal embedder of 101.4 MB:

| Lever              | Saves  | Costs                                                                        |
| ------------------ | ------ | ---------------------------------------------------------------------------- |
| `icu-root` feature | 9.1 MB | `Intl` formats in the root locale unless `icu.skpk` ships beside the payload |
| no `webgpu`        | 5.3 MB | WebGPU                                                                       |
| no `audio`         | 5.1 MB | Web Audio                                                                    |
| no `lottie`        | 2.9 MB | `<lottie>` and `<sk-sticker>`                                                |
| `no-devtools`      | 1.6 MB | F12 and the devtools protocol                                                |

Every feature, what the app sees without it, and feature sets for common programs: [Features and size](/embedding/features/).

## macOS owns the main thread

AppKit needs the process main thread, so on macOS `spawn` returns `Error::MainThreadRequired`. `run_on_main_thread` runs the app there and keeps the thread until the app closes:

```rust
fn main() {
    let builder = skathi::Builder::new()
        .payload_file(payload_path())
        .setup(|host| {
            // No Handle comes back on macOS: take it here for whatever outlives setup.
            start_background_services(host.handle());
            Ok(())
        });

    #[cfg(target_os = "macos")]
    if let Err(e) = builder.run_on_main_thread(|exit| {
        stop_background_services(); // runs after the app's shutdown hooks
        i32::from(!exit.clean)       // the process's exit code
    }) {
        eprintln!("{e}");
        std::process::exit(2);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let ui = builder.spawn().unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
        std::process::exit(if ui.wait().clean { 0 } else { 1 });
    }
}
```

- Your work runs on threads you start, before `run_on_main_thread` or from `setup`.
- The callback runs on a separate thread after shutdown (or its deadline): stop workers, flush logs, return the exit code. A panic in it exits with code 1.
- Startup errors return normally, so the same `eprintln!` path handles both platforms.

On Linux, frames for `<sk-surface>` are CPU BGRA; on macOS they can be Metal textures, IOSurfaces or CVPixelBuffers. See [Native frames](/embedding/native-frames/#platform-support).

## Before you ship

- The payload was built by the `sk` of the crate's version.
- `cargo build --release` with `prod`; the exe starts with no `SK_*` variables set.
- The profile path is where your program keeps its data, and `storage_encrypted` is what you want.
- On Windows, the manifest keeps Common Controls 6, and the exe was linked with the crate's link arguments.
- On macOS, `run_on_main_thread`; on Linux, `desktop_id` matches your desktop entry.

Next: [Troubleshooting](/embedding/troubleshooting/).
