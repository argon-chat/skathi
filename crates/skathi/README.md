# skathi

Run a [Skathi](https://skathi.land) app (Vue 3 or Svelte 5, rendered by gpui, scripted in
JavaScriptCore) inside your own Rust executable.

```toml
[dependencies]
skathi = "0.3"
```

```rust,ignore
let ui = skathi::Builder::new()
    .payload_file("app/dist/release/.sk/app.skpk")
    .module("hello", |m| {
        m.function("greet", |who: String| format!("hello, {who}"));
    })
    .spawn()?;
ui.wait();
```

The app imports it as `import { greet } from "native:hello"`.

The app's payload is built with the `sk` CLI from `@skathi-native/cli` (`bunx sk publish
--payload-only`). The [crate documentation](https://docs.rs/skathi) covers the payload, the build
prerequisites (the static CRT and `clang-cl` on Windows) and the build script an executable needs.

Licensed under MIT or Apache-2.0, at your option.
