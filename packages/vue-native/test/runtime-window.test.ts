// The tier-3 windowing namespace: `import { appWindow } from "vue-native"`.
//
// The implementation itself lives in the host's JS prelude
// (crates/vn-app/src/window.prelude.js), because the state snapshot and the
// tier-1 globals it is built on have to exist before the app bundle evaluates.
// What runtime.js owns — and what these tests pin — is the NAMESPACE: that
// apps reach the surface through an import rather than a global, and that a
// host without the prelude degrades loudly instead of silently.

import { afterEach, describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

afterEach(() => {
  delete globalThis.__VN_WINDOW__;
});

/** The shape the host prelude publishes. */
function fakeHost() {
  const calls = [];
  const appWindow = {
    maximize: () => calls.push("maximize"),
    isMaximized: () => true,
  };
  return {
    calls,
    host: {
      appWindow,
      defineWindow: (o) => calls.push(["defineWindow", o]),
      getCurrentWindow: () => appWindow,
    },
  };
}

describe("appWindow namespace", () => {
  test("re-exports the host's appWindow rather than a copy of it", () => {
    const { calls, host } = fakeHost();
    globalThis.__VN_WINDOW__ = host;
    const rt = createRuntime();

    expect(rt.appWindow).toBe(host.appWindow);
    rt.appWindow.maximize();
    expect(calls).toEqual(["maximize"]);
    expect(rt.advisories()).toEqual([]);
  });

  test("getCurrentWindow() is appWindow — the name ported Tauri code uses", () => {
    const { host } = fakeHost();
    globalThis.__VN_WINDOW__ = host;
    const rt = createRuntime();
    // Both spellings must stay valid when multi-window lands: `getCurrentWindow()`
    // keeps meaning "this window" and `appWindow` keeps being its binding.
    expect(rt.getCurrentWindow()).toBe(rt.appWindow);
  });

  test("defineWindow forwards to the host", () => {
    const { calls, host } = fakeHost();
    globalThis.__VN_WINDOW__ = host;
    const rt = createRuntime();
    rt.defineWindow({ title: "Notes" });
    expect(calls).toEqual([["defineWindow", { title: "Notes" }]]);
  });
});

describe("no windowing host", () => {
  test("the advisory is LAZY — importing the module warns about nothing", () => {
    const rt = createRuntime();
    // An advisory that fires on code nobody wrote is noise: an embedder that
    // never touches `appWindow` must never hear about it.
    expect(rt.advisories()).toEqual([]);
  });

  test("touching the inert window warns once and never throws", () => {
    const rt = createRuntime();
    // Verbs stay callable…
    expect(() => rt.appWindow.maximize()).not.toThrow();
    // …predicates still answer a boolean…
    expect(rt.appWindow.isMaximized()).toBe(false);
    // …and assignment is swallowed rather than raising.
    expect(() => {
      rt.appWindow.backdrop = "mica";
    }).not.toThrow();

    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    expect(advisories[0]).toContain("windowing host is missing");
    expect(advisories[0]).toContain("docs/WINDOWING.md");
  });

  test("defineWindow without a host warns instead of throwing", () => {
    const rt = createRuntime();
    expect(() => rt.defineWindow({ title: "x" })).not.toThrow();
    expect(rt.advisories().length).toBe(1);
  });
});
