// The generated window prelude, evaluated whole against stub `__sk_window_*` host functions: the change
// notifications behind appWindow's reactivity, and the cancelable `close-requested`.

import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { computed } from "@vue/runtime-core";
import { createRuntime } from "./runtime-harness";

const PRELUDE = readFileSync(join(import.meta.dir, "../../../crates/sk-app/src/window.prelude.js"), "utf8");

const windowSlot = globalThis as { __SK_WINDOW__?: unknown };
afterEach(() => {
  delete windowSlot.__SK_WINDOW__;
});

interface Booted {
  host: SkWindowHost & { onChange: (fn: () => void) => () => void };
  global: EventTarget;
  state: Record<string, unknown>;
  verbs: string[];
  observe(kinds: string): boolean;
}

function boot(title = "One"): Booted {
  const state: Record<string, unknown> = {
    title,
    maximized: false,
    minimized: false,
    visible: true,
    focused: true,
    innerWidth: 800,
    innerHeight: 600,
    devicePixelRatio: 1,
    outer: { x: 0, y: 0, width: 800, height: 600 },
  };
  const verbs: string[] = [];
  let observer: ((kinds: string, json: string) => boolean) | null = null;
  const global = Object.assign(new EventTarget(), { innerWidth: 0 }) as EventTarget & { __SK_WINDOW__?: unknown };
  const document = Object.assign(new EventTarget(), { documentElement: null });
  const host: Record<string, unknown> = {
    __sk_window_install_controls: (window: SkAppWindow) => {
      window.show = () => {
        verbs.push("show");
      };
      window.hide = () => {
        verbs.push("hide");
      };
    },
    __sk_window_state: () => JSON.stringify(state),
    __sk_window_observe: (fn: typeof observer) => {
      observer = fn;
    },
    __sk_window_set_title: () => {},
    __sk_window_verb: (verb: string) => verbs.push(verb),
    __sk_window_advise: () => {},
    __sk_advise: () => true,
    __sk_window_match_media: () => '{"matches":false}',
  };
  new Function("globalThis", "document", "requestAnimationFrame", ...Object.keys(host), PRELUDE)(
    global,
    document,
    (cb: () => void) => setTimeout(cb, 0),
    ...Object.values(host),
  );
  return {
    host: global.__SK_WINDOW__ as Booted["host"],
    global,
    state,
    verbs,
    observe: (kinds) => observer!(kinds, JSON.stringify(state)),
  };
}

describe("window prelude change notifications", () => {
  test("title trims and collapses only HTML ASCII whitespace, keeping all other UTF-16 code units", () => {
    const w = boot("\t  initial\f\n title \r");
    expect(w.host.appWindow.title).toBe("initial title");
    for (const [value, expected] of [
      ["", ""],
      ["\t\n\f\r ", ""],
      [" a\tb\nc\fd\re  f ", "a b c d e f"],
      [" \u00a0\t\u000b ", "\u00a0 \u000b"],
      ["\ud800\t\ud83d\ude00\n\udfff\u2028", "\ud800 \ud83d\ude00 \udfff\u2028"],
    ] as const) {
      w.host.appWindow.title = value;
      expect(w.host.appWindow.title).toBe(expected);
    }
  });

  test("onChange runs for a new snapshot, a title change and a refresh that sees new state — and not otherwise", () => {
    const w = boot();
    let changes = 0;
    w.host.onChange(() => changes++);
    w.observe("sync");
    expect(changes).toBe(0);
    w.state.maximized = true;
    w.observe("sync");
    expect(changes).toBe(1);
    w.observe("maximize");
    expect(changes).toBe(1);
    w.host.appWindow.title = "Two";
    w.host.appWindow.title = "Two";
    expect(changes).toBe(2);
    w.state.focused = false;
    expect(w.host.appWindow.isFocused()).toBe(false);
    expect(changes).toBe(3);
    expect(w.host.appWindow.isFocused()).toBe(false);
    expect(changes).toBe(3);
  });

  test("through the runtime, a computed over title and isMaximized() follows the host", () => {
    const w = boot();
    windowSlot.__SK_WINDOW__ = w.host;
    const rt = createRuntime();
    expect(rt.appWindow as unknown).toBe(w.host.appWindow);
    const label = computed(() => `${rt.appWindow.title}:${rt.appWindow.isMaximized()}`);
    expect(label.value).toBe("One:false");
    w.state.maximized = true;
    w.observe("maximize");
    expect(label.value).toBe("One:true");
    rt.appWindow.title = "Two";
    expect(label.value).toBe("Two:true");
  });
});

describe("close-requested", () => {
  test("uncancelled, the close goes ahead and pagehide fires", () => {
    const w = boot();
    let hidden = 0;
    w.global.addEventListener("pagehide", () => hidden++);
    let asked = 0;
    w.host.appWindow.addEventListener("close-requested", () => asked++);
    expect(w.observe("close-requested")).toBe(true);
    expect(asked).toBe(1);
    expect(hidden).toBe(1);
  });

  test("cancelled, the window stays and the app can hide it, then show it again", () => {
    const w = boot();
    let hidden = 0;
    w.global.addEventListener("pagehide", () => hidden++);
    w.host.appWindow.addEventListener("close-requested", (e) => {
      e.preventDefault();
      w.host.appWindow.hide();
    });
    expect(w.observe("close-requested")).toBe(false);
    expect(w.observe("close-requested")).toBe(false);
    expect(hidden).toBe(0);
    expect(w.verbs).toEqual(["hide", "hide"]);
    w.host.appWindow.show();
    expect(w.verbs).toEqual(["hide", "hide", "show"]);
  });

  test("close() is not asked", () => {
    const w = boot();
    let asked = 0;
    w.host.appWindow.addEventListener("close-requested", (e) => {
      asked++;
      e.preventDefault();
    });
    w.host.appWindow.close();
    expect(asked).toBe(0);
    expect(w.verbs).toEqual(["close"]);
  });

  test("a beforeunload veto without sticky activation does not keep the window", () => {
    const w = boot();
    w.global.addEventListener("beforeunload", (e) => e.preventDefault());
    expect(w.observe("close-requested")).toBe(true);
  });
});
