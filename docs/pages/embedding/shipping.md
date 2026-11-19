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
- Asset packs (`fonts.skpk`, `assets.skpk`, `icu.skpk`) sit next to it and are found there, and so does an AOT payload's native image, `app.image.dll` (since 0.6).

</div>
<div class="tradeoff" data-tone="gain">

**Inside the exe: `payload_static`**

- One file to ship, sign and verify.
- `payload_static(include_bytes!("../app/dist/release/.sk/app.skpk"))` borrows the bytes with no copy.
- Every app change means a Rust rebuild, and the payload is part of the exe's signature.
- An AOT payload's `app.image.dll` ships next to the exe (since 0.6); see [What you ship](/guides/aot/#what-you-ship).

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
