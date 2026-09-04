# vn-test fixtures

Apps that exist for a spec rather than for a demo. `apps/` is the shelf of
things worth looking at; this is the shelf of things worth *asserting*, and the
two get in each other's way when they share a directory — an app here may be
eight lines and have a deliberately broken configuration.

`launch({ app: "packages/vn-test/fixtures/<name>/src/main.ts" })` — `resolveEntry`
takes a path as readily as a name (docs/TESTING.md §2).

| fixture | what it is for |
|---|---|
| `devtools-on` / `devtools-off` | the m22 devtools switch (docs/DEVTOOLS.md). Twins: identical code, and their `vn.config.json` differs by one key. `devtools-off` declares the top-level `"devtools": false`, which is the static spelling of `appDevtools.disable()`. Both publish `appDevtools` as a global, because `eval` on the devtools wire runs in GLOBAL scope and a module export is not a global in either boot mode. |
