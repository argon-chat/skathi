# vue-native

Desktop-app framework: **Vue 3** SFCs → **gpui** (Zed's GPU UI, D3D11 on Windows) with the
JS running in **JavaScriptCore** via [redjs](https://github.com/argon-chat/redjs)
(Bun's JSC prebuilt, ~3 ns host calls). Windows-first (native Acrylic/Mica window chrome),
macOS later.

- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Research corpus (feasibility spikes, gpui/redjs/Vue deep-dives): `research/reports/`

## Prerequisites

- Rust 1.97.1 MSVC (pinned by `rust-toolchain.toml`), VS Build Tools + Windows SDK
- LLVM (`clang-cl` on PATH) — redjs compiles its JSC glue with it
- bun (JS tooling)
- `git submodule update --init` then `vendor/redjs/setup.ps1` (fetches the ~1.1 GB
  bun-webkit prebuilt) and `bun run fetch:yoga` inside it

Build with an explicit target: `cargo build --target x86_64-pc-windows-msvc`
(see `.cargo/config.toml` for why).
