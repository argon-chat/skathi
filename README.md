<p align="center">
  <img src="logos/full_side_text_logo.svg" width="480" alt="Skathi">
</p>

# Skathi

Desktop apps from Vue 3 and Svelte 5 components, without a browser. GPU-rendered UI with shader
effects from CSS, JavaScriptCore for the script, or the script compiled ahead of time to machine code.
Windows, macOS (arm64) and Linux (x64).

Documentation: [skathi.land](https://skathi.land)

## Quick start

Needs [bun](https://bun.sh).

```sh
bun create skathi my-app
cd my-app
bun install
bun run dev
```

It asks for a template (`showcase`, `minimal` or `svelte`), the platforms to package for and the
package manager. `bun create skathi my-app --template minimal --yes` takes the answers as options.

## Commands

The app's scripts run them (`bun run dev`, `bun run publish`); `bunx sk` runs any of them directly.

| Command      | What it does                                        |
| ------------ | --------------------------------------------------- |
| `sk create`  | a new app                                           |
| `sk dev`     | run the app with hot reload                         |
| `sk build`   | a debug build with devtools                         |
| `sk publish` | the release build (`--aot`: compiled ahead of time) |
| `sk check`   | what will not run, or will be slow, under AOT       |
| `sk prune`   | clear cached workloads                              |

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
