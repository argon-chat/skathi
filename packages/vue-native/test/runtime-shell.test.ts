// Links, window.open and `appShell`: the JS half. Pinned here: which clicks follow a link, that `preventDefault()`
// anywhere on the path cancels it, that content reaches only http/https/mailto, and that `appShell` turns the settle
// protocol into promises that reject with the right names.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

function linkWorld(href: string | null, options: Parameters<typeof createRuntime>[0] = {}) {
  const rt = createRuntime(options);
  const root = rt.nodeOps.createElement("root");
  const a = rt.nodeOps.createElement("a");
  const span = rt.nodeOps.createElement("span");
  rt.nodeOps.insert(a, root, null);
  rt.nodeOps.insert(span, a, null);
  if (href !== null) rt.patchProp(a, "href", null, href);
  const opens = () => rt.host.desktop.calls.filter((c: { op: string }) => c.op === "openExternal");
  const click = (node: { id: number }) => rt.host.dispatcher(node.__vnId, rt.EV_CODES.click, 5, 6, 0, 0, undefined);
  return { rt, root, a, span, opens, click };
}

describe("<a href>: the click bit follows the href", () => {
  test("an href arms the host's click bit with no @click at all", () => {
    const { rt, a } = linkWorld("https://example.com/");
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: a.__vnId, code: 1, on: true }]);
  });

  test("removing the last @click keeps a link listening; removing the href disarms it", () => {
    const { rt, a } = linkWorld("https://example.com/");
    const fn = () => {};
    rt.patchProp(a, "onClick", null, fn);
    rt.patchProp(a, "onClick", fn, null);
    expect(rt.flagsFor(rt.EV_CODES.click).at(-1)).toEqual({ id: a.__vnId, code: 1, on: true });
    rt.patchProp(a, "href", "https://example.com/", null);
    expect(rt.flagsFor(rt.EV_CODES.click).at(-1)).toEqual({ id: a.__vnId, code: 1, on: false });
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
    expect(opens()[0].args[0]).toBe("mailto:someone@example.com");
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
    // gpui fires on every listening box under the pointer; vn-tree's `bind_events` lets only the innermost through.
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
    expect(rt.advisories()[0]).toContain("RouterLink");
  });

  test("file:, custom protocols and ms-settings: are refused before the host", () => {
    for (const href of ["file:///C:/Windows/System32/calc.exe", "myapp://open", "ms-settings:privacy"]) {
      const { rt, a, opens, click } = linkWorld(href);
      click(a);
      expect(opens()).toEqual([]);
      expect(rt.advisories().length).toBe(1);
      expect(rt.advisories()[0]).toContain("appShell.openExternal");
    }
  });

  test("javascript:void(0) is silent; other javascript: URLs advise and never run", () => {
    const quiet = linkWorld("javascript:void(0)");
    quiet.click(quiet.a);
    expect(quiet.rt.advisories()).toEqual([]);
    const loud = linkWorld("javascript:alert(1)");
    loud.click(loud.a);
    expect(loud.opens()).toEqual([]);
    expect(loud.rt.advisories()[0]).toContain("not executed");
  });

  test("<a download href=blob:…> asks the host to save it (Save As), with no gesture needed", () => {
    const { rt, a, opens, click } = linkWorld("blob:vn/1234");
    rt.patchProp(a, "download", null, "report.csv");
    click(a);
    expect(opens()).toEqual([]);
    const saves = rt.host.desktop.fs.calls.filter((c: { op: string }) => c.op === "download");
    expect(saves.map((c: { args: unknown[] }) => c.args)).toEqual([["blob:vn/1234", "report.csv"]]);
    expect(rt.advisories().filter((l: string) => l.includes("Save As dialog")).length).toBe(1);
  });

  test("<a download href=https:…> opens the browser — a cross-origin download is ignored there too", () => {
    const { rt, a, opens, click } = linkWorld("https://example.com/f.zip");
    rt.patchProp(a, "download", null, "");
    click(a);
    expect(opens().length).toBe(1);
  });

  test("a failed open is a console warning, not an exception", async () => {
    const { rt, a, click } = linkWorld("mailto:x@example.com");
    click(a);
    rt.host.desktop.settle(1, false, {
      name: "NotFoundError",
      message: "no application is associated",
      code: "ENOASSOC",
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(rt.host.logs.some((l: string) => l.includes("could not open") && l.includes("no application"))).toBe(true);
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
    expect(rt.global.open("https://example.com/x", "_blank", "noopener")).toBe(null);
    expect(opens()).toEqual([{ op: "openExternal", args: ["https://example.com/x", true] }]);
  });

  test("another scheme is refused; no url / about:blank are ignored", () => {
    const { rt, opens } = linkWorld(null);
    expect(rt.global.open("myapp://x")).toBe(null);
    expect(rt.advisories()[0]).toContain("appShell.openExternal");
    expect(rt.global.open()).toBe(null);
    expect(rt.global.open("about:blank")).toBe(null);
    expect(rt.global.open("/relative")).toBe(null);
    expect(rt.global.open("C:\\x.txt")).toBe(null);
    expect(opens()).toEqual([]);
  });

  test("the prelude's open is the fallback for everything that is not external", () => {
    const seen: unknown[][] = [];
    const prelude = (...args: unknown[]) => (seen.push(args), null);
    const rt = createRuntime({ globals: { open: prelude } });
    expect(rt.global.open("/relative", "_self")).toBe(null);
    expect(rt.global.open()).toBe(null);
    expect(rt.global.open("https://example.com/")).toBe(null);
    expect(seen).toEqual([
      ["/relative", "_self", undefined],
      [undefined, undefined, undefined],
    ]);
  });
});

describe("appShell", () => {
  test("each verb crosses as its host fn and resolves when the pump settles it", async () => {
    const { rt } = linkWorld(null);
    const calls = [
      rt.appShell.openExternal("myapp://x"),
      rt.appShell.openPath("C:\\a\\b.txt"),
      rt.appShell.openPath("C:\\a\\setup.exe", { allowExecutable: true }),
      rt.appShell.showItemInFolder("C:\\a\\b.txt"),
      rt.appShell.trashItem("C:\\a\\b.txt"),
    ];
    expect(rt.host.desktop.calls).toEqual([
      { op: "openExternal", args: ["myapp://x", false] },
      { op: "openPath", args: ["C:\\a\\b.txt", false] },
      { op: "openPath", args: ["C:\\a\\setup.exe", true] },
      { op: "showItem", args: ["C:\\a\\b.txt"] },
      { op: "trash", args: ["C:\\a\\b.txt"] },
    ]);
    for (let t = 1; t <= 5; t++) rt.host.desktop.settle(t, true, {});
    expect(await Promise.all(calls)).toEqual([undefined, undefined, undefined, undefined, undefined]);
  });

  test("an async failure rejects with the host's name, code and osError", async () => {
    const { rt } = linkWorld(null);
    const p = rt.appShell.openPath("C:\\a\\missing.txt");
    rt.host.desktop.settle(1, false, { name: "NotFoundError", message: "gone", code: "ENOENT", osError: 2 });
    const e = await p.catch((x: unknown) => x);
    expect(e).toBeInstanceOf(Error);
    expect(e.name).toBe("NotFoundError");
    expect(e.message).toBe("gone");
    expect(e.code).toBe("ENOENT");
    expect(e.osError).toBe(2);
  });

  test("a synchronous refusal rejects at once; a TypeError stays a TypeError", async () => {
    const { rt } = linkWorld(null);
    rt.host.desktop.refuse = { error: 'openPath: "a.txt" is not an absolute path', name: "TypeError" };
    const e = await rt.appShell.openPath("a.txt").catch((x: unknown) => x);
    expect(e).toBeInstanceOf(TypeError);
    rt.host.desktop.refuse = { error: "it would run it", name: "NotAllowedError", code: "EEXECUTABLE" };
    const f = await rt.appShell.openPath("C:\\x\\setup.exe").catch((x: unknown) => x);
    expect([f.name, f.code]).toEqual(["NotAllowedError", "EEXECUTABLE"]);
  });

  test("with no desktop host it is inert: one advisory, every call rejects, links advise", async () => {
    const { rt, a, click } = linkWorld("https://example.com/", { desktop: false });
    const e = await rt.appShell.openPath("C:\\a.txt").catch((x: unknown) => x);
    expect(e.name).toBe("NotSupportedError");
    await rt.appShell.trashItem("C:\\a.txt").catch(() => {});
    click(a);
    const adv = rt.advisories();
    expect(adv.filter((l: string) => l.includes("`appShell` is inert")).length).toBe(1);
    expect(adv.some((l: string) => l.includes("the link was not opened"))).toBe(true);
  });
});
