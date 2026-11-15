<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="icon.svg">
    <img src="icon-black.svg" width="128" alt="Skathi">
  </picture>
</p>

# Skathi

Desktop apps from Vue 3 single-file components, without a browser. GPU-rendered UI, JavaScriptCore
for the script. Windows, macOS (arm64) and Linux (x64).

## Quick start

Needs [bun](https://bun.sh).

```sh
bunx @skathi-native/cli@beta create my-app
cd my-app
bun install
bun run dev
```

## Commands

| Command      | What it does                 |
| ------------ | ---------------------------- |
| `sk create`  | a new app                    |
| `sk dev`     | run the app with hot reload  |
| `sk build`   | a debug build with devtools  |
| `sk publish` | the release build            |
| `sk check`   | what in the app will not run |
| `sk prune`   | clear cached workloads       |

`sk <command> --help` lists its options.

## Building from source

Rust (pinned by `rust-toolchain.toml`), LLVM, bun, then:

```sh
git submodule update --init
vendor/redjs/setup.ps1        # setup.sh on macOS and Linux
bun install
cargo build -p sk-app --target <triple>   # x86_64-pc-windows-msvc, aarch64-apple-darwin, x86_64-unknown-linux-gnu
```

## License

[Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.

The host statically links JavaScriptCore from Bun's WebKit build, parts of which are LGPL-2. Its
source is at [oven-sh/WebKit](https://github.com/oven-sh/WebKit), at the commit named in
`vendor/redjs/pins.txt`. To run an app on a modified JavaScriptCore, rebuild the workload against it
(`sk workload build`) and stamp the app onto it (`sk build --workload <exe>`).
