# Skathi: instructions for agents

Vue 3 SFCs rendered by gpui, with the JS running in JavaScriptCore (redjs). Windows-first; macOS
and Linux hosts exist.

## Layout

- `crates/sk-app`: the host (window, event loop, JS bindings, embedding API `sk_app::embed`).
- `crates/sk-runtime`, `sk-tree`, `sk-css`, `sk-elements`: JS runtime, element tree, CSS, elements.
- `crates/sk-desktop`, `sk-permissions`, `sk-storage`, `sk-clipboard`, …: platform services.
- `packages/runtime` (`@skathi-native/runtime`): the JS runtime apps import, run in JavaScriptCore.
- `packages/cli` (`@skathi-native/cli`): SFC compiler, `prod.ts` bundler, the `sk` CLI (`cli/sk.ts`), devtools.
- `packages/test` (`@skathi-native/test`): end-to-end specs that drive a real host.
- `scripts/`: repo tooling (preludes, shaders, release hosts, AOT).
- `apps/*`: sample and test apps (`apps/hello` is the main one).

## Commands

Build with an explicit target (see `.cargo/config.toml`):

```
cargo build --target x86_64-pc-windows-msvc -p sk-app
cargo test --target x86_64-pc-windows-msvc -p <crate>
```

JS, from the repo root (one bun workspace):

```
bun install
bun run test             # runtime and CLI unit tests
bun run typecheck        # needs Node on PATH: vue-tsc run by bun cannot resolve .vue imports
bun run preludes         # regenerate crates/sk-app/src/{prelude,window.prelude}.js
bun run preludes:check
bun run shaders:check
```

End-to-end: `bun test specs` in `packages/test`, after `cargo build -p sk-app`.

`bun run sweep` removes incremental caches and artifacts in `target/` that no recent build has
used (`--dry-run` to see the size first); cargo never does. Keep logs and scratch files out of `target/`.

A new crate dependency must pass `cargo deny check licenses` (`deny.toml`): no GPL family.

Formatting, from the repo root: `bun fmt` (`bun fmt --check` to verify). Settings are in `bunfig.toml`,
exclusions in `.prettierignore`. Rust is `cargo fmt`'s.

## Rules

- **Comments only when necessary.** Write one when the code cannot say it: a non-obvious reason,
  an invariant, a platform quirk, a `// SAFETY:` on `unsafe`. Do not restate what the code does,
  narrate history ("used to", "until m26", "after the audit"), cite milestones, reports or
  documents, or leave essays. Doc comments on public API are one or two lines. When in doubt,
  leave it out.
- **No generated documents.** Do not add design notes, plans, reports, audits or summaries as
  `.md` files; explain a change in its commit message. README, LICENSE and the usual repo files
  (CONTRIBUTING, SECURITY, CHANGELOG) are fine when a person asks for them.
- **Strict TypeScript.** Every TS project extends `tsconfig.base.json` (`strict` plus `noUncheckedIndexedAccess`,
  `exactOptionalPropertyTypes`, `noUnused*` and the rest); `bun run typecheck` checks all of them. No `any`, no
  `@ts-ignore`/`@ts-expect-error`; `unknown` and narrowing at boundaries.
- **Web and platform APIs are Rust.** A new API (or a rewrite of one) is a native class registered from
  Rust (redjs `define_class`, host data, native promises), not a TS module in `packages/runtime`. TS is
  only for what must be JS: the Vue and Svelte renderer glue, the module loader and HMR, and objects that
  libraries patch directly. A thin TS shim over a host function is not a reason to write it in TS.
  A piece may be TS when performance calls for it: code that is all promises, timers and callbacks
  into JS, or a call so small that crossing into the host costs more than the work. Keep that piece
  as small as the reason for it, and the rest of the API in Rust.
- **Generated files are not edited by hand.** `crates/sk-app/src/prelude.js` and
  `window.prelude.js` come from the `.ts` beside them (`bun run preludes`).
- **This is a public SDK.** No product-specific code or policy: an app's own features live in
  the embedder (`sk_app::embed::HostContext` hooks, sk.config.json `"aliases"`).
- Error and advisory messages say what is wrong and what to do, without pointing at files a
  user of the SDK does not have.
- **Docs landing: card rows go through `CardRow` and `CardStack`** (`docs/src/components`), and the
  columns of a row end flush; balance the content rather than leave a stretched card half empty.
  Section intros go through `SectionIntro` and use the whole row (two text columns, or text and an
  aside that end flush; centred only above a symmetric section). `bun run check:layout` in `docs/`
  checks both in a headless Chromium.
- Commits: `type(scope): summary` (`fix(css): …`, `feat(window): …`).
