// Links and window.open. Pinned here: which clicks follow a link, that `preventDefault()` anywhere on the path
// cancels it, and that content reaches only http/https/mailto.

import { describe, expect, test } from "bun:test";
import { createRuntime, type RealmGlobal } from "./runtime-harness";

const winOpen = (rt: { global: RealmGlobal }) =>
  rt.global.open as (url?: string, target?: string, features?: string) => unknown;

function linkWorld(href: string | null, options: Parameters<typeof createRuntime>[0] = {}) {
  const rt = createRuntime(options);
  const root = rt.nodeOps.createElement("root");
  const a = rt.nodeOps.createElement("a");
  const span = rt.nodeOps.createElement("span");
  rt.nodeOps.insert(a, root, null);
  rt.nodeOps.insert(span, a, null);
  if (href !== null) rt.patchProp(a, "href", null, href);
  const opens = () => rt.host.desktop.calls.filter((c: { op: string }) => c.op === "openExternal");
  const click = (node: { __skId: number }) => rt.host.dispatcher(node.__skId, rt.EV_CODES.click, 5, 6, 0, 0, undefined);
  return { rt, root, a, span, opens, click };
}

describe("<a href>: the click bit follows the href", () => {
  test("an href arms the host's click bit with no @click at all", () => {
    const { rt, a } = linkWorld("https://example.com/");
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: a.__skId, code: 1, on: true }]);
  });

  test("removing the last @click keeps a link listening; removing the href disarms it", () => {
    const { rt, a } = linkWorld("https://example.com/");
    const fn = () => {};
    rt.patchProp(a, "onClick", null, fn);
    rt.patchProp(a, "onClick", fn, null);
    expect(rt.flagsFor(rt.EV_CODES.click).at(-1)).toEqual({ id: a.__skId, code: 1, on: true });
    rt.patchProp(a, "href", "https://example.com/", null);
    expect(rt.flagsFor(rt.EV_CODES.click).at(-1)).toEqual({ id: a.__skId, code: 1, on: false });
  });

  test("an <a> without an href is not a link", () => {
    const { rt } = linkWorld(null);
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([]);
  });
});

describe("<a href>: activation", () => {
  test("a host click on an https link hands the serialised URL to the shell as CONTENT", () => {
    const { a, opens, click } = linkWorld("https://example.com/a b?q=1");
    click(a);
    expect(opens()).toEqual([{ op: "openExternal", args: ["https://example.com/a%20b?q=1", true] }]);
  });

  test("a click on a child bubbles to the link", () => {
    const { span, opens, click } = linkWorld("mailto:someone@example.com");
    click(span);
    expect(opens().length).toBe(1);
    expect(opens()[0]?.args[0]).toBe("mailto:someone@example.com");
  });

  test("preventDefault anywhere on the path cancels the navigation (RouterLink's shape)", () => {
    const { rt, a, span, opens, click } = linkWorld("https://example.com/");
    rt.patchProp(a, "onClick", null, (e: { preventDefault(): void }) => e.preventDefault());
    click(span);
    expect(opens()).toEqual([]);
  });

  test("a stopped click still navigates — stopPropagation is not preventDefault", () => {
    const { rt, span, opens, click } = linkWorld("https://example.com/");
    rt.patchProp(span, "onClick", null, (e: { stopPropagation(): void }) => e.stopPropagation());
    click(span);
    expect(opens().length).toBe(1);
  });

  test("each host click is one navigation (the host delivers one click per press)", () => {
    // gpui fires on every listening box under the pointer; sk-tree's `bind_events` lets only the innermost through.
    const { rt, span, opens, click } = linkWorld("https://example.com/");
    rt.patchProp(span, "onClick", null, () => {});
    click(span);
    click(span);
    expect(opens().length).toBe(2);
  });

  test("a <button> inside a link is the button's click, not the link's", () => {
    const { rt, a, opens, click } = linkWorld("https://example.com/");
    const button = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(button, a, null);
    rt.patchProp(button, "onClick", null, () => {});
    click(button);
    expect(opens()).toEqual([]);
  });

  test("relative, #hash and empty hrefs do nothing; a relative one advises once", () => {
    for (const href of ["#top", "", "   "]) {
      const { rt, a, opens, click } = linkWorld(href);
      click(a);
      expect(opens()).toEqual([]);
      expect(rt.advisories()).toEqual([]);
    }
    const { rt, a, opens, click } = linkWorld("/settings");
    click(a);
    click(a);
    expect(opens()).toEqual([]);
    expect(rt.advisories().length).toBe(1);
  });

  test("file:, custom protocols and ms-settings: are refused before the host", () => {
    for (const href of ["file:///C:/Windows/System32/calc.exe", "myapp://open", "ms-settings:privacy"]) {
      const { rt, a, opens, click } = linkWorld(href);
      click(a);
      expect(opens()).toEqual([]);
      expect(rt.advisories().length).toBe(1);
    }
  });

  test("javascript:void(0) is silent; other javascript: URLs advise and never run", () => {
    const quiet = linkWorld("javascript:void(0)");
    quiet.click(quiet.a);
    expect(quiet.rt.advisories()).toEqual([]);
    const loud = linkWorld("javascript:alert(1)");
    loud.click(loud.a);
    expect(loud.opens()).toEqual([]);
  });

  test("<a download href=blob:…> asks the host to save it (Save As), with no gesture needed", () => {
    const { rt, a, opens, click } = linkWorld("blob:sk/1234");
    rt.patchProp(a, "download", null, "report.csv");
    click(a);
    expect(opens()).toEqual([]);
    const saves = rt.host.desktop.fs.calls.filter((c: { op: string }) => c.op === "download");
    expect(saves.map((c: { args: unknown[] }) => c.args)).toEqual([["blob:sk/1234", "report.csv"]]);
  });

  test("<a download href=https:…> opens the browser — a cross-origin download is ignored there too", () => {
    const { rt, a, opens, click } = linkWorld("https://example.com/f.zip");
    rt.patchProp(a, "download", null, "");
    click(a);
    expect(opens().length).toBe(1);
  });
});

describe("el.click() and dispatchEvent", () => {
  test("a.click() follows the link, through the same walk a real click takes", () => {
    const { rt, a, opens } = linkWorld("https://example.com/");
    const seen: string[] = [];
    rt.patchProp(a, "onClick", null, (e: { type: string }) => seen.push(e.type));
    a.click();
    expect(seen).toEqual(["click"]);
    expect(opens().length).toBe(1);
  });

  test("two a.click() calls in one turn are two navigations, as in a browser", () => {
    const { a, opens } = linkWorld("https://example.com/");
    a.click();
    a.click();
    expect(opens().length).toBe(2);
  });

  test("click() on a disabled button fires nothing", () => {
    const rt = createRuntime();
    const b = rt.nodeOps.createElement("button");
    let n = 0;
    rt.patchProp(b, "onClick", null, () => n++);
    rt.patchProp(b, "disabled", null, true);
    b.click();
    expect(n).toBe(0);
    rt.patchProp(b, "disabled", true, false);
    b.click();
    expect(n).toBe(1);
  });

  test("click() from inside its own click handler is ignored (click-in-progress)", () => {
    const rt = createRuntime();
    const b = rt.nodeOps.createElement("button");
    let n = 0;
    rt.patchProp(b, "onClick", null, () => {
      n++;
      b.click();
    });
    b.click();
    expect(n).toBe(1);
  });

  test("dispatchEvent(new MouseEvent('click')) activates; a CustomEvent('click') does not", () => {
    class FakeMouseEvent extends Event {}
    const { rt, a, opens } = linkWorld("https://example.com/", { MouseEvent: FakeMouseEvent });
    a.dispatchEvent(new CustomEvent("click", { bubbles: true, cancelable: true }));
    expect(opens()).toEqual([]);
    a.dispatchEvent(new FakeMouseEvent("click", { bubbles: true, cancelable: true }));
    expect(opens().length).toBe(1);
    expect(rt.advisories()).toEqual([]);
  });
});

describe("window.open", () => {
  test("an external URL opens the system browser and returns null", () => {
    const { rt, opens } = linkWorld(null);
    expect(winOpen(rt)("https://example.com/x", "_blank", "noopener")).toBe(null);
    expect(opens()).toEqual([{ op: "openExternal", args: ["https://example.com/x", true] }]);
  });

  test("another scheme is refused; no url / about:blank are ignored", () => {
    const { rt, opens } = linkWorld(null);
    expect(winOpen(rt)("myapp://x")).toBe(null);
    expect(winOpen(rt)()).toBe(null);
    expect(winOpen(rt)("about:blank")).toBe(null);
    expect(winOpen(rt)("/relative")).toBe(null);
    expect(winOpen(rt)("C:\\x.txt")).toBe(null);
    expect(opens()).toEqual([]);
  });

  test("the prelude's open is the fallback for everything that is not external", () => {
    const seen: unknown[][] = [];
    const prelude = (...args: unknown[]) => (seen.push(args), null);
    const rt = createRuntime({ globals: { open: prelude } });
    expect(winOpen(rt)("/relative", "_self")).toBe(null);
    expect(winOpen(rt)()).toBe(null);
    expect(winOpen(rt)("https://example.com/")).toBe(null);
    expect(seen).toEqual([
      ["/relative", "_self", undefined],
      [undefined, undefined, undefined],
    ]);
  });
});
