# vue-native

Desktop-app framework: **Vue 3** SFCs → **gpui** (Zed's GPU UI, D3D11 on Windows) with the
JS running in **JavaScriptCore** via [redjs](https://github.com/argon-chat/redjs)
(Bun's JSC prebuilt, ~3 ns host calls). Windows-first (native Acrylic/Mica window chrome),
with macOS and Linux hosts.

## Packaging an app

The CLI needs bun, not Rust: the hosts it runs and stamps come from the GitHub release of its
version (`vn host fetch`, or on demand) and are cached; `--host <exe>` stamps one of your own.

```sh
vn create     # a new app, asked for step by step
vn dev        # in the app: run it with hot reload
vn build      # a debug exe: devtools in, not minified (dist/debug)
vn publish    # the release exe (dist/release)
```

`vn publish` leaves devtools out unless `vn.config.json` selects the engine parts and keeps them.

## Building from source

### Prerequisites

- Rust 1.97.1 MSVC (pinned by `rust-toolchain.toml`), VS Build Tools + Windows SDK
- LLVM (`clang-cl` on PATH) — redjs compiles its JSC glue with it
- bun (JS tooling)
- `git submodule update --init` then `vendor/redjs/setup.ps1` (fetches the ~1.1 GB
  bun-webkit prebuilt); the lean build vue-native uses needs no vendored yoga

### Dev loop

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

## Tailwind CSS

Standard Tailwind utility classes work in `.vue` templates — compiled by the
**real** Tailwind JIT, matched by the same class matcher that runs your own
`<style scoped>`:

```html
<div
  class="flex flex-col gap-2 p-4 rounded-xl bg-white/10
            hover:bg-white/25 transition-colors duration-300"
></div>
```

An app opts in by dropping a `tailwind.config.js` next to its `src/`
(`apps/hello/tailwind.config.js`); without one the builders emit exactly what
they emitted before. Both builders run Tailwind themselves — no PostCSS step to
wire up, no extra watcher:

- **dev** — the utilities sheet is written as a synthetic module,
  `dist/modules/.vn__tailwind.css.js` (id `.vn/tailwind.css`), which rides the
  ordinary `setCss` transport. Its id sorts first, so utilities land ahead of
  every component `<style>` in the aggregate. Adding a class that was never used
  before re-runs Tailwind and ships a **`css-only`** change: the app restyles
  without a rerender, a reload, or any loss of component state.
- **prod** — the same sheet is pushed into `__VN_CSS__` ahead of the SFC styles.

**Pinned to Tailwind v3.4, deliberately.** v3 core utilities emit literal
declarations (`padding: 1rem`, `background-color: rgb(255 255 255 / 0.1)`) that
`crates/vn-css` maps straight onto gpui. v4 routes nearly everything through
`var(--tw-*)` + `calc()` + `@property`, which the engine does not resolve yet —
a v4 bump would degrade almost every utility. The rationale, the ordering
decision, and what v4 would need from vn-css live in
[`packages/vue-native/tailwind.ts`](packages/vue-native/tailwind.ts); the
per-app config choices (preflight off, `*Opacity` core plugins off) are
documented inline in [`apps/hello/tailwind.config.js`](apps/hello/tailwind.config.js).

Utilities the engine cannot render never fail silently — vn-css prints a
one-shot advisory per gap on every stylesheet compile (`[css] …` on stderr).

## Languages

`navigator.language` and `navigator.languages` are the OS's display languages, and every
`fetch`, `XMLHttpRequest` and WebSocket sends a matching `Accept-Language`, as Chrome does.

The host's own dialogs (the permission prompt, the crash dialog) ship in 29 languages
([`crates/vn-locale/locales`](crates/vn-locale/locales)). `appLocale` picks the same way for an
app's strings, and lets an app choose its language or translate the SDK's:

```ts
import { appLocale } from "vue-native";

appLocale.define({
  en: { "app.greeting": "Hello, {name}" },
  de: { "app.greeting": "Hallo, {name}" },
  hy: { "vn.consent.allow": "&Թույլատրել" },
});
appLocale.t("app.greeting", { name: "Ada" }); // the best match for navigator.languages
appLocale.language = "de"; // the app's choice, for its text and the SDK's dialogs; null follows the OS
```

## License

vue-native is licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.

The host (`vn-app`) statically links JavaScriptCore, WTF and bmalloc from Bun's WebKit build
through [redjs](https://github.com/argon-chat/redjs). Parts of JavaScriptCore and WTF are
licensed under the GNU LGPL 2; the rest of WebKit is BSD. To meet the LGPL, the patched WebKit
source is public at [oven-sh/WebKit](https://github.com/oven-sh/WebKit), at the commit named by
`webkit.version` in `vendor/redjs/pins.txt`, and the host is built from this repository: to run
an app on a modified JavaScriptCore, build that commit, rebuild the host against it
(`vn host build`) and stamp the app onto it (`vn build --host <exe>`). An app's own code is a
payload the host reads at startup, not linked into it.

Other native code in the host: ICU (Unicode License V3), mimalloc (MIT), SQLite3MultipleCiphers
(MIT; SQLite is public domain), libvpx (BSD-3-Clause), ThorVG (MIT), gpui (Apache-2.0) and the
Rust crates in `Cargo.lock`, which `cargo deny check licenses` keeps to permissive licenses and
MPL-2.0 (`deny.toml`).
