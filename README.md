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
  bun-webkit prebuilt); the lean build vue-native uses needs no vendored yoga

## Dev loop

```powershell
cd packages/vue-native
bun install
bun build.ts --watch          # SFC compiler + module registry + HMR watcher
cd ../..
cargo build --target x86_64-pc-windows-msvc -p vn-app
./target/x86_64-pc-windows-msvc/debug/vn-app.exe
```

Edit `apps/hello/src/*.vue` while the app runs: template edits hot-patch with
component state preserved, style edits restyle in place, script edits reload
the component. The window is a Mica-backdrop gpui window; the app is plain
browser-flavored Vue (`<script setup>`, `<style scoped>`, `v-model`, `@click`).

Build with an explicit target: `cargo build --target x86_64-pc-windows-msvc`
(see `.cargo/config.toml` for why).
