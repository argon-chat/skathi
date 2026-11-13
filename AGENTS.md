# vue-native: instructions for agents

Vue 3 SFCs rendered by gpui, with the JS running in JavaScriptCore (redjs). Windows-first; macOS
and Linux hosts exist.

## Layout

- `crates/vn-app`: the host (window, event loop, JS bindings, embedding API `vn_app::embed`).
- `crates/vn-runtime`, `vn-tree`, `vn-css`, `vn-elements`: JS runtime, element tree, CSS, elements.
- `crates/vn-desktop`, `vn-permissions`, `vn-storage`, `vn-clipboard`, …: platform services.
- `packages/vue-native`: SFC compiler, `prod.ts` bundler, the `vn` CLI (`cli/vn.ts`), devtools.
- `packages/vn-test`: end-to-end specs that drive a real host.
- `apps/*`: sample and test apps (`apps/hello` is the main one).

## Commands

Build with an explicit target (see `.cargo/config.toml`):

```
cargo build --target x86_64-pc-windows-msvc -p vn-app
cargo test --target x86_64-pc-windows-msvc -p <crate>
```

In `packages/vue-native`:

```
bun install
bun test                 # unit tests
bun run typecheck
bun run preludes         # regenerate crates/vn-app/src/{prelude,window.prelude}.js
bun run preludes:check
bun run shaders:check
```

End-to-end: `bun test specs` in `packages/vn-test`, after `cargo build -p vn-app`.

Formatting, from the repo root: `bun fmt` (`bun fmt --check` to verify). Settings are in `bunfig.toml`,
exclusions in `.prettierignore`. Rust is `cargo fmt`'s.

## Rules

- **Comments only when necessary.** Write one when the code cannot say it: a non-obvious reason,
  an invariant, a platform quirk, a `// SAFETY:` on `unsafe`. Do not restate what the code does,
  narrate history ("used to", "until m26", "after the audit"), cite milestones, reports or
  documents, or leave essays. Doc comments on public API are one or two lines. When in doubt,
  leave it out.
- **No documentation files.** `README.md` is the only document in this repo. Do not add `.md`
  files, design notes or reports; explain a change in its commit message.
- **Generated files are not edited by hand.** `crates/vn-app/src/prelude.js` and
  `window.prelude.js` come from the `.ts` beside them (`bun run preludes`).
- **This is a public SDK.** No product-specific code or policy: an app's own features live in
  the embedder (`vn_app::embed::HostContext` hooks, vn.config.json `"aliases"`).
- Error and advisory messages say what is wrong and what to do, without pointing at files a
  user of the SDK does not have.
- Commits: `type(scope): summary` (`fix(css): …`, `feat(window): …`).
