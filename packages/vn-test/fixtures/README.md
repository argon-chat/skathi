# vn-test fixtures

Apps that exist for a spec rather than for a demo. `apps/` is the shelf of
things worth looking at; this is the shelf of things worth *asserting*, and the
two get in each other's way when they share a directory — an app here may be
eight lines and have a deliberately broken configuration.

`launch({ app: "packages/vn-test/fixtures/<name>/src/main.ts" })` — `resolveEntry`
takes a path as readily as a name (docs/TESTING.md §2).

| fixture | what it is for |
|---|---|
| `measure-on-mount` | the m22 "a DOM layout read lays out" fix (`specs/measure.spec.ts`). `apps/meet-probe`'s join card in twelve lines: measure a child from `onMounted` + `nextTick` and write the answer back as the parent's `height`, with an `overflow: hidden` beside it. It reports what `scrollHeight` answered at every stage a real app measures from — `onMounted`, `nextTick`, the first rAF, `setTimeout(0)`, and one late enough that a frame has certainly been painted — so the spec can pin all five at the browser's number rather than at 0. |
| `devtools-on` / `devtools-off` | the m22 devtools switch (docs/DEVTOOLS.md). Twins: identical code, and their `vn.config.json` differs by one key. `devtools-off` declares the top-level `"devtools": false`, which is the static spelling of `appDevtools.disable()`. Both publish `appDevtools` as a global, because `eval` on the devtools wire runs in GLOBAL scope and a module export is not a global in either boot mode. |
| `shell` | links, `window.open` and `appShell` (`specs/shell.spec.ts`, docs/SHELL.md). One link per shape the tier-1 path distinguishes — https with a listening child (one press must be one click and one navigation), mailto, a custom scheme and `file:` (refused), a cancelled one — and `__VN_TEST_SHELL__.run(verb, …)`, which records how each `appShell` promise settled, because a promise does not survive the eval round trip. Run under `VN_SHELL=record`, so nothing reaches the OS. |
