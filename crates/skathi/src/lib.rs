//! Run a Skathi app inside your own Rust executable.
//!
//! Skathi renders Vue 3 and Svelte 5 apps with gpui and runs their script in JavaScriptCore. The
//! app's JS half is a payload (`.skpk`) built by the `sk` CLI; this crate is the native half. Your
//! `main` hands the payload to a [`Builder`], registers native modules the app imports
//! (`import { greet } from "native:hello"`) and gets a [`Handle`] to send it events.
//!
//! # A minimal embedder
//!
//! ```no_run
//! // Counts the Rust heap for the devtools `mem` command; optional.
//! #[global_allocator]
//! static GLOBAL: skathi::HostAllocator = skathi::HostAllocator;
//!
//! #[skathi::data]
//! struct Greeting {
//!     text: String,
//! }
//!
//! struct Hello;
//!
//! // `import { greet, on } from "native:hello"` in the app.
//! #[skathi::module(events(ready = u32))]
//! impl Hello {
//!     pub fn greet(&self, who: String) -> Greeting {
//!         Greeting { text: format!("hello, {who}") }
//!     }
//! }
//!
//! fn main() {
//!     // F12 starts this executable again as the inspector (see `inspector_exe` below).
//!     if std::env::args().nth(1).as_deref() == Some("--devtools") {
//!         return skathi::run_exe();
//!     }
//!     let builder = skathi::Builder::new()
//!         .payload_file("app/dist/release/.sk/app.skpk")
//!         .inspector_exe(std::env::current_exe().unwrap())
//!         .module("hello", Hello);
//!     #[cfg(target_os = "macos")]
//!     if let Err(e) = builder.run_on_main_thread(|exit| i32::from(!exit.clean)) {
//!         eprintln!("{e}");
//!         std::process::exit(2);
//!     }
//!     #[cfg(not(target_os = "macos"))]
//!     {
//!         let ui = builder.spawn().unwrap_or_else(|e| {
//!             eprintln!("{e}");
//!             std::process::exit(2)
//!         });
//!         ui.emit_to("hello", "ready", &std::process::id()).ok();
//!         std::process::exit(if ui.wait().clean { 0 } else { 1 });
//!     }
//! }
//! ```
//!
//! [`module`] exports every `pub fn` taking `&self` under its camelCase name: it runs inside the JS
//! call and returns a value, or, marked `#[task]`, runs on a worker and returns a promise. For a few
//! ad-hoc functions, a closure `|m: &mut skathi::Module| …` registers them instead. Arguments and
//! results are [`data`] types (serde, plus [`Type`] with the `types` feature, on by default), and
//! `sk types --exe <program>` writes the app's `native-modules.d.ts` from them. Deriving [`Type`]
//! directly needs `#[specta(crate = skathi::specta)]` on the type.
//!
//! [`Builder::payload_static`] takes `include_bytes!` of the payload instead, for a single-file
//! executable. [`Builder::setup`] runs before any app JS with a [`HostContext`]: direct
//! JavaScriptCore access ([`js`]), cookies, OS-sealed secrets and frame, reload and shutdown hooks.
//! [`tray`] and [`hotkeys`] are the native tray and system-wide hotkeys. [`surface`] hands frames
//! a native renderer draws (a D3D11 texture on Windows, CPU BGRA anywhere) to `<sk-surface>` elements.
//!
//! # The app payload
//!
//! The app is an ordinary Skathi project, made and built with the `@skathi-native/cli` package
//! (needs [bun](https://bun.sh)):
//!
//! ```sh
//! bun create skathi app --template minimal --yes
//! cd app && bun install
//! bunx sk build --payload-only     # debug: app/dist/debug/.sk/app.skpk
//! bunx sk publish --payload-only   # release: app/dist/release/.sk/app.skpk
//! ```
//!
//! The CLI and this crate are released together and must have the same version: the payload
//! carries the JS half of the host protocol, and a mismatch is refused with [`Error::Payload`].
//! An app split into asset packs (`fonts.skpk`, `assets.skpk`) keeps them beside `app.skpk`;
//! [`Builder::payload_file`] finds them there. `SK_PAYLOAD=<path>` names a payload when the
//! builder is given none.
//!
//! # Building
//!
//! The first build downloads the pinned JavaScriptCore prebuilt (about 1.6 GB unpacked on Windows),
//! mimalloc, SQLite3 Multiple Ciphers and a ThorVG archive, and checks each against its hash.
//! `REDJS_CACHE_DIR` and `SK_THORVG_CACHE_DIR` move their caches. bun is not needed to build the
//! Rust side.
//!
//! On Windows (x86_64 MSVC) the build needs the MSVC tools and Windows SDK, `clang-cl` from LLVM on
//! `PATH`, and the static CRT, which redjs's build checks for. In `.cargo/config.toml`:
//!
//! ```toml
//! [target.x86_64-pc-windows-msvc]
//! rustflags = ["-C", "target-feature=+crt-static"]
//! ```
//!
//! On macOS (arm64) it needs Xcode with its Metal toolchain (or the `runtime-shaders` feature) and
//! `MACOSX_DEPLOYMENT_TARGET=13.0` or later. On Linux (x86_64), clang and the development
//! packages of Wayland, X11 (xcb, xkbcommon), fontconfig and ALSA.
//!
//! The executable has to carry linker arguments and, on Windows, a manifest (Common Controls 6,
//! per-monitor DPI awareness) that a library cannot add to it. This crate reports them to the
//! embedder's build script, which passes them on (with `embed-resource` as a build dependency):
//!
//! ```rust,ignore
//! // build.rs
//! fn main() {
//!     for arg in std::env::var("DEP_SKATHI_LINK_ARGS").unwrap().split_whitespace() {
//!         println!("cargo:rustc-link-arg={arg}");
//!     }
//!     if let Ok(dir) = std::env::var("DEP_SKATHI_WINDOWS_RESOURCES") {
//!         let rc = std::path::Path::new(&dir).join("gpui.rc");
//!         embed_resource::compile(rc, embed_resource::ParamsIncludeDirs([&dir]))
//!             .manifest_required()
//!             .unwrap();
//!     }
//! }
//! ```
//!
//! An application with its own manifest uses that instead, and must keep the Common Controls 6
//! dependency: without it Windows refuses to start the executable.
//!
//! Skathi's own builds patch `async-task` to the merged but unreleased waker identity fix
//! (smol-rs/async-task#108), which keeps some idle tasks from being polled again and again. An
//! application can do the same until it is released:
//!
//! ```toml
//! [patch.crates-io]
//! async-task = { git = "https://github.com/smol-rs/async-task", rev = "41fd3359c75e221bd8b187a8ee361aa309c1714c" }
//! ```
//!
//! # Features
//!
//! On by default: `prod` (runs a payload; off, the host boots the dev output named by `SK_DIST`),
//! `types` (native module declarations, with specta), `audio` (Web Audio), `storages` (localStorage, cookies and secrets on disk), `clipboard`,
//! `indexeddb`, `webgpu` and `native-dx12`. Off: `battery` (the Battery Status API),
//! `no-devtools` (compiles out the devtools protocol and the inspector) and `runtime-shaders`
//! (macOS: compile the Metal shaders at startup).
//!
//! # License
//!
//! MIT or Apache-2.0. The executable statically links JavaScriptCore from Bun's WebKit build,
//! parts of which are LGPL-2; its source is at <https://github.com/oven-sh/WebKit>, at the commit
//! redjs pins.

/// The serde `#[skathi::data]` derives with.
pub use sk_app::embed::serde;
pub use sk_app::embed::{
    encrypt_local_storage_at, surface, Builder, Call, D3D11Device, D3D12Device, D3D12Fence,
    Declared, EmitError, Error, Exit, Function, Handle, Handler, HostContext, HostCookie,
    IntoModule, Job, Module, Reply, SameSite, SharedTexture, SurfaceError, SurfaceEvent,
    SurfaceHandle, SurfaceInfo, SurfaceStats, SurfaceWatch, Typed,
};
#[cfg(feature = "types")]
pub use sk_app::embed::{specta, Type};
pub use sk_app::embed::{MetalDevice, MetalEvent};
pub use sk_app::{hotkeys, js, run_exe, tray, HostAllocator};
pub use skathi_macros::{data, module};
