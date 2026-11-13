// Forms, focus and location.reload(): the JS half.
//
// The host half of focus (`__vn_focus`) is crates/vn-tree/src/render.rs; reload's (`__vn_reload`) is
// crates/vn-app/src/reload.rs. Pinned here is what only the runtime can get wrong: which clicks and Enter submit
// which form, with which submitter; the order and targets of the four focus events; what `document.activeElement`
// says; and what a reload undoes in the realm it keeps.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Node = { id: number; attrs: Record<string, string> };
type Rt = ReturnType<typeof createRuntime>;

/** `<root><form><input/><button/></form></root>`, plus a submit log. */
function formWorld(options: { button?: string | null; fields?: number; novalidate?: boolean } = {}) {
  const rt = createRuntime();
  const root = rt.nodeOps.createElement("div");
  const form = rt.nodeOps.createElement("form");
  rt.nodeOps.insert(form, root, null);
  if (options.novalidate !== false) rt.patchProp(form, "novalidate", null, "");
  const fields: Node[] = [];
  for (let i = 0; i < (options.fields ?? 1); i++) {
    const input = rt.nodeOps.createElement("input");
    rt.patchProp(input, "type", null, "email");
    rt.nodeOps.insert(input, form, null);
    fields.push(input);
  }
  let button: Node | null = null;
  if (options.button !== null) {
    button = rt.nodeOps.createElement("button");
    if (options.button) rt.patchProp(button, "type", null, options.button);
    rt.nodeOps.insert(button, form, null);
  }
  const submits: { submitter: unknown; cancelable: boolean; bubbles: boolean; type: string }[] = [];
  let cancel = true;
  rt.patchProp(form, "onSubmit", null, (e: { submitter: unknown; cancelable: boolean; bubbles: boolean; type: string; preventDefault(): void }) => {
    submits.push({ submitter: e.submitter, cancelable: e.cancelable, bubbles: e.bubbles, type: e.type });
    if (cancel) e.preventDefault();
  });
  const click = (n: Node) => rt.host.dispatcher(n.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
  const enter = (n: Node) => rt.host.dispatcher(n.__vnId, rt.EV_CODES.keydown, 0, 0, 0, 0, "Enter");
  const navigations = () => rt.advisories().filter((l: string) => l.includes("no document to navigate")).length;
  return {
    rt, root, form, fields, field: fields[0], button, submits, click, enter, navigations,
    keep: () => {
      cancel = false;
    },
  };
}

/** A submit log entry's submitter, by node id (mirror nodes are cyclic; never deep-compare them). */
const who = (s: { submitter: unknown }) => (s.submitter as Node | null)?.__vnId ?? null;

const lastFlag = (rt: Rt, code: number, id: number) =>
  rt.host.listeners.filter((l: { id: number; code: number }) => l.code === code && l.id === id).at(-1)?.on;

describe("forms: what the host is asked to deliver", () => {
  test("@submit is kept, with no 'not delivered' advisory", () => {
    const { rt } = formWorld();
    expect(rt.advisories()).toEqual([]);
  });

  test("a <button> listens for its own click; type=button stops it, type=submit restarts it", () => {
    const { rt, button } = formWorld();
    expect(lastFlag(rt, rt.EV_CODES.click, button!.__vnId)).toBe(true);
    rt.patchProp(button!, "type", null, "button");
    expect(lastFlag(rt, rt.EV_CODES.click, button!.__vnId)).toBe(false);
    rt.patchProp(button!, "type", "button", "submit");
    expect(lastFlag(rt, rt.EV_CODES.click, button!.__vnId)).toBe(true);
  });

  test("a text-like <input> hears Enter; a checkbox does not", () => {
    const { rt, field } = formWorld();
    expect(lastFlag(rt, rt.EV_CODES.keydown, field.__vnId)).toBe(true);
    rt.patchProp(field, "type", "email", "checkbox");
    expect(lastFlag(rt, rt.EV_CODES.keydown, field.__vnId)).toBe(false);
  });
});

describe("forms: a submit button's activation", () => {
  test("a host click submits the form owner with the button as submitter: a cancelable, bubbling SubmitEvent", () => {
    const { rt, button, submits, click } = formWorld();
    click(button!);
    expect(submits.map(who)).toEqual([button!.__vnId]);
    expect(submits[0]).toMatchObject({ cancelable: true, bubbles: true, type: "submit" });
    expect(rt.global.SubmitEvent).toBeFunction();
  });

  test("a click on something inside the button reaches the button's activation", () => {
    const { rt, button, submits, click } = formWorld();
    const icon = rt.nodeOps.createElement("span");
    rt.nodeOps.insert(icon, button!, null);
    rt.patchProp(icon, "onClick", null, () => {});
    click(icon);
    expect(submits.length).toBe(1);
    expect(submits[0].submitter).toBe(button);
  });

  test("a disabled button, a type=button and a cancelled click do not submit", () => {
    const w = formWorld();
    w.rt.patchProp(w.button!, "disabled", null, "");
    w.click(w.button!);
    (w.button as unknown as { click(): void }).click();
    expect(w.submits).toEqual([]);
    const b = formWorld({ button: "button" });
    b.click(b.button!);
    expect(b.submits).toEqual([]);
    const c = formWorld();
    c.rt.patchProp(c.button!, "onClick", null, (e: { preventDefault(): void }) => e.preventDefault());
    c.click(c.button!);
    expect(c.submits).toEqual([]);
  });

  test("a disabled <fieldset> disables the button inside it, but not one in its first <legend>; and says what it cannot do", () => {
    const { rt, form, button, submits, click } = formWorld();
    const fieldset = rt.nodeOps.createElement("fieldset");
    const legend = rt.nodeOps.createElement("legend");
    const inLegend = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(fieldset, form, null);
    rt.nodeOps.insert(legend, fieldset, null);
    rt.nodeOps.insert(inLegend, legend, null);
    rt.nodeOps.insert(button!, fieldset, null);
    rt.patchProp(fieldset, "disabled", null, "");
    click(button!);
    expect(submits).toEqual([]);
    click(inLegend);
    expect(submits.map(who)).toEqual([inLegend.__vnId]);
    expect(rt.advisories().filter((l: string) => l.includes("<fieldset disabled>")).length).toBe(1);
  });

  test("`:disabled=\"false\"` is not disabled", () => {
    const { rt, button, submits, click } = formWorld();
    rt.patchProp(button!, "disabled", null, false);
    click(button!);
    expect(submits.length).toBe(1);
  });

  test("an uncancelled submission says it navigates nowhere; a cancelled one says nothing", () => {
    const w = formWorld();
    w.click(w.button!);
    expect(w.navigations()).toBe(0);
    w.keep();
    w.click(w.button!);
    expect(w.navigations()).toBe(1);
  });

  test("a button outside any form does nothing; the form attribute names an owner elsewhere", () => {
    const { rt, root, form, submits } = formWorld({ button: null });
    const outside = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(outside, root, null);
    rt.host.dispatcher(outside.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    expect(submits).toEqual([]);
    rt.patchProp(form, "id", null, "login");
    rt.patchProp(outside, "form", null, "login");
    rt.host.dispatcher(outside.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
    expect(submits.map(who)).toEqual([outside.__vnId]);
  });
});

describe("forms: implicit submission (Enter)", () => {
  test("Enter clicks the default button, which submits with it as submitter", () => {
    const { rt, button, field, submits, enter } = formWorld();
    let clicks = 0;
    rt.patchProp(button!, "onClick", null, () => clicks++);
    enter(field);
    expect(clicks).toBe(1);
    expect(submits.map(who)).toEqual([button!.__vnId]);
  });

  test("a disabled default button means Enter does nothing at all", () => {
    const { rt, button, field, submits, enter } = formWorld({ fields: 1 });
    rt.patchProp(button!, "disabled", null, "");
    enter(field);
    expect(submits).toEqual([]);
  });

  test("the FIRST submit button is the default; a type=button before it is not", () => {
    const { rt, form, field, button, submits, enter } = formWorld();
    const plain = rt.nodeOps.createElement("button");
    rt.patchProp(plain, "type", null, "button");
    rt.nodeOps.insert(plain, form, field as never);
    enter(field);
    expect(submits.map(who)).toEqual([button!.__vnId]);
  });

  test("no submit button: one field submits the form itself (submitter null), two fields do nothing", () => {
    const one = formWorld({ button: null, fields: 1 });
    one.enter(one.field);
    expect(one.submits.map(who)).toEqual([null]);
    const two = formWorld({ button: null, fields: 2 });
    two.enter(two.field);
    expect(two.submits).toEqual([]);
  });

  test("a cancelled keydown, another key, and a field outside a form do not submit", () => {
    const { rt, root, field, submits, enter } = formWorld();
    rt.patchProp(field, "onKeydown", null, (e: { preventDefault(): void }) => e.preventDefault());
    enter(field);
    rt.host.dispatcher(field.__vnId, rt.EV_CODES.keydown, 0, 0, 0, 0, "a");
    const loose = rt.nodeOps.createElement("input");
    rt.nodeOps.insert(loose, root, null);
    enter(loose);
    expect(submits).toEqual([]);
  });
});

describe("forms: requestSubmit(), submit(), reset()", () => {
  test("requestSubmit() fires submit with a null submitter; requestSubmit(button) names it", () => {
    const { form, button, submits } = formWorld();
    (form as unknown as { requestSubmit(s?: unknown): void }).requestSubmit();
    (form as unknown as { requestSubmit(s?: unknown): void }).requestSubmit(button);
    expect(submits.map(who)).toEqual([null, button!.__vnId]);
  });

  test("requestSubmit's TypeError and NotFoundError, and only a form has it", () => {
    const { rt, form, field } = formWorld();
    const f = form as unknown as { requestSubmit(s?: unknown): void };
    expect(() => f.requestSubmit(field)).toThrow(TypeError);
    const other = formWorld();
    let err: { name?: string } | null = null;
    try {
      f.requestSubmit(other.button);
    } catch (e) {
      err = e as { name?: string };
    }
    expect(err?.name).toBe("NotFoundError");
    const div = rt.nodeOps.createElement("div") as unknown as { requestSubmit(): void };
    expect(() => div.requestSubmit()).toThrow(TypeError);
  });

  test("submit() fires no event and says it navigates nowhere", () => {
    const { form, submits, navigations } = formWorld();
    (form as unknown as { submit(): void }).submit();
    expect(submits).toEqual([]);
    expect(navigations()).toBe(1);
  });

  test("a submit from inside a submit handler is ignored, as HTML's firing flag says", () => {
    const { rt, form, button, click } = formWorld();
    let n = 0;
    rt.patchProp(form, "onSubmit", null, (e: { preventDefault(): void }) => {
      n++;
      e.preventDefault();
      (form as unknown as { requestSubmit(): void }).requestSubmit();
    });
    click(button!);
    expect(n).toBe(1);
  });

  test("a reset button fires reset, and says the controls are not restored", () => {
    const { rt, form, button, click } = formWorld({ button: "reset" });
    let resets = 0;
    rt.patchProp(form, "onReset", null, () => resets++);
    click(button!);
    expect(resets).toBe(1);
    expect(rt.advisories().some((l: string) => l.includes("cannot restore the controls"))).toBe(true);
  });

  test("a form WITHOUT novalidate says once that constraints are not checked; novalidate is silent", () => {
    const w = formWorld({ novalidate: false });
    w.rt.patchProp(w.field, "required", null, "");
    w.click(w.button!);
    w.click(w.button!);
    expect(w.rt.advisories().filter((l: string) => l.includes("WITHOUT constraint validation")).length).toBe(1);
    expect(w.submits.length).toBe(2);
    const quiet = formWorld();
    quiet.rt.patchProp(quiet.field, "required", null, "");
    quiet.click(quiet.button!);
    expect(quiet.rt.advisories()).toEqual([]);
  });
});

describe("focus: el.focus(), el.blur(), document.activeElement", () => {
  /** Two inputs, every focus event logged as `type@node(related)`. */
  function focusWorld() {
    const rt = createRuntime();
    const root = rt.nodeOps.createElement("div");
    const a = rt.nodeOps.createElement("input");
    const b = rt.nodeOps.createElement("input");
    rt.nodeOps.insert(a, root, null);
    rt.nodeOps.insert(b, root, null);
    const name = (n: unknown) => (n === a ? "a" : n === b ? "b" : String(n));
    const log: string[] = [];
    for (const [n, label] of [[a, "a"], [b, "b"]] as const) {
      for (const type of ["focus", "blur", "focusin", "focusout"]) {
        rt.patchProp(n, `on${type[0].toUpperCase()}${type.slice(1)}`, null, (e: { relatedTarget?: unknown; type: string }) =>
          log.push(`${e.type}@${label}(${name(e.relatedTarget ?? null)})`),
        );
      }
    }
    // focusin/focusout bubble: the root hears them too.
    rt.patchProp(root, "onFocusin", null, (e: { target: unknown }) => log.push(`focusin^root:${name(e.target)}`));
    return { rt, root, a, b, log };
  }
  const focus = (n: unknown) => (n as { focus(): void }).focus();
  const blur = (n: unknown) => (n as { blur(): void }).blur();

  test("a synchronous move fires blur, focusout, focus, focusin, with relatedTarget, before focus() returns", () => {
    const { rt, a, b, log } = focusWorld();
    rt.host.focus.answer = (id: number) => (id === a.__vnId ? "moved:0" : `moved:${a.__vnId}`);
    focus(a);
    expect(rt.global.document.activeElement).toBe(a);
    log.length = 0;
    focus(b);
    expect(log).toEqual([
      "blur@a(b)",
      "focusout@a(b)",
      "focus@b(a)",
      "focusin@b(a)",
      "focusin^root:b",
    ]);
    expect(rt.global.document.activeElement).toBe(b);
  });

  test("'already' and 'refused' fire nothing and change nothing", () => {
    const { rt, a, log } = focusWorld();
    rt.host.focus.answer = () => "refused";
    focus(a);
    rt.host.focus.answer = () => "already";
    focus(a);
    expect(log).toEqual([]);
    expect(rt.advisories()).toEqual([]);
  });

  test("'pending' fires nothing now and says so once; the host's focus event then moves activeElement", () => {
    const { rt, a, log } = focusWorld();
    rt.host.focus.answer = () => "pending";
    focus(a);
    focus(a);
    expect(log).toEqual([]);
    expect(rt.advisories().filter((l: string) => l.includes("next frame")).length).toBe(1);
    expect(rt.global.document.activeElement).toBe(null);
    rt.host.dispatcher(a.__vnId, rt.EV_CODES.focus, 0, 0, 0, 0, undefined);
    expect(rt.global.document.activeElement).toBe(a);
    expect(log).toEqual(["focus@a(null)", "focusin@a(null)", "focusin^root:a"]);
    rt.host.dispatcher(a.__vnId, rt.EV_CODES.blur, 0, 0, 0, 0, undefined);
    expect(rt.global.document.activeElement).toBe(null);
  });

  test("blur() on the focused element fires blur and focusout; on another one, nothing", () => {
    const { rt, a, b, log } = focusWorld();
    rt.host.focus.answer = (id: number, op: number) => (op === 0 ? "moved:0" : id === a.__vnId ? "blurred" : "");
    focus(a);
    log.length = 0;
    blur(b);
    expect(log).toEqual([]);
    blur(a);
    expect(log).toEqual(["blur@a(null)", "focusout@a(null)"]);
    expect(rt.global.document.activeElement).toBe(null);
  });

  test("a disabled control and a detached node never reach the host", () => {
    const { rt, a } = focusWorld();
    rt.patchProp(a, "disabled", null, "");
    focus(a);
    const loose = rt.global.document.createElement("input");
    focus(loose);
    expect(rt.host.focus.calls).toEqual([]);
  });
});

describe("focus: a handler that moves the focus abandons the move it interrupts", () => {
  const focus = (n: unknown) => (n as { focus(): void }).focus();
  const blur = (n: unknown) => (n as { blur(): void }).blur();
  function nestedWorld() {
    const rt = createRuntime();
    const root = rt.nodeOps.createElement("div");
    const nodes: Record<string, Node> = {};
    for (const id of ["inside", "trigger", "other"]) {
      nodes[id] = rt.nodeOps.createElement("button");
      rt.nodeOps.insert(nodes[id], root, null);
    }
    const name = (n: unknown) => Object.keys(nodes).find((k) => nodes[k] === n) ?? "null";
    // With nothing focused, `activeElement` is the root: `body` (this harness has none).
    const active = () => Object.keys(nodes).find((k) => nodes[k] === rt.global.document.activeElement) ?? "body";
    const log: string[] = [];
    // `rt.patchProp` keeps one handler per event: tests add theirs through `on`.
    const extra: Record<string, ((e: unknown) => void)[]> = {};
    for (const [id, n] of Object.entries(nodes)) {
      for (const type of ["focus", "blur", "focusin", "focusout"]) {
        rt.patchProp(n, `on${type[0].toUpperCase()}${type.slice(1)}`, null, (e: { relatedTarget?: unknown; type: string }) => {
          log.push(`${e.type}@${id}(${name(e.relatedTarget ?? null)}) active=${active()}`);
          for (const f of extra[`${id}:${type}`]?.splice(0) ?? []) f(e);
        });
      }
    }
    const on = (id: string, type: string, f: (e: unknown) => void) => (extra[`${id}:${type}`] ??= []).push(f);
    // The host's slot, as `js_focus` / `js_blur` keep it.
    let slot = 0;
    rt.host.focus.answer = (id: number, op: number) => {
      if (op === 1) {
        if (slot !== id) return "";
        slot = 0;
        return "blurred";
      }
      if (slot === id) return "already";
      const old = slot;
      slot = id;
      return `moved:${old}`;
    };
    const start = () => {
      focus(nodes.inside);
      log.length = 0;
    };
    return { rt, nodes, log, on, start, active, slot: () => slot };
  }

  test("reka's trap: a focusout handler that pulls the focus back keeps it from ever reaching the target", () => {
    const { nodes, log, on, start, active } = nestedWorld();
    start();
    on("inside", "focusout", (e) => {
      const rel = (e as { relatedTarget: unknown }).relatedTarget;
      if (rel !== null && rel !== nodes.inside) focus(nodes.inside);
    });
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focusout@inside(trigger) active=body",
      "focus@inside(null) active=inside",
      "focusin@inside(null) active=inside",
    ]);
    expect(active()).toBe("inside");
  });

  test("a blur handler that moves the focus: focusout follows with no relatedTarget", () => {
    const { nodes, log, on, start, active } = nestedWorld();
    start();
    on("inside", "blur", () => focus(nodes.other));
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focus@other(null) active=other",
      "focusin@other(null) active=other",
      "focusout@inside(null) active=other",
    ]);
    expect(active()).toBe("other");
  });

  test("a focus handler that moves the focus cancels its own node's focusin", () => {
    const { nodes, log, on, start, active } = nestedWorld();
    start();
    on("trigger", "focus", () => focus(nodes.other));
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focusout@inside(trigger) active=body",
      "focus@trigger(inside) active=trigger",
      "blur@trigger(other) active=body",
      "focusout@trigger(other) active=body",
      "focus@other(trigger) active=other",
      "focusin@other(trigger) active=other",
    ]);
    expect(active()).toBe("other");
  });

  test("a focus handler that blurs its own node", () => {
    const { nodes, log, on, start, active } = nestedWorld();
    start();
    on("trigger", "focus", () => blur(nodes.trigger));
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focusout@inside(trigger) active=body",
      "focus@trigger(inside) active=trigger",
      "blur@trigger(null) active=body",
      "focusout@trigger(null) active=body",
    ]);
    expect(active()).toBe("body");
  });

  test("focus() on the node being moved to, from the blur phase, focuses it at once", () => {
    const { nodes, log, on, start, active } = nestedWorld();
    start();
    on("inside", "focusout", () => focus(nodes.trigger));
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focusout@inside(trigger) active=body",
      "focus@trigger(null) active=trigger",
      "focusin@trigger(null) active=trigger",
    ]);
    expect(active()).toBe("trigger");
  });

  test("blur() on the node being moved to, from the blur phase, does nothing", () => {
    const { rt, nodes, log, on, start, active, slot } = nestedWorld();
    start();
    on("inside", "focusout", () => blur(nodes.trigger));
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focusout@inside(trigger) active=body",
      "focus@trigger(inside) active=trigger",
      "focusin@trigger(inside) active=trigger",
    ]);
    expect(active()).toBe("trigger");
    // …and the host was never asked: its focus stays where JS says it is.
    expect(rt.host.focus.calls.filter((c: { op: number }) => c.op === 1)).toEqual([]);
    expect(slot()).toBe(nodes.trigger.__vnId);
  });

  test("a blur handler that takes the focus back for the old node", () => {
    const { nodes, log, on, start, active } = nestedWorld();
    start();
    on("inside", "blur", () => focus(nodes.inside));
    focus(nodes.trigger);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focus@inside(null) active=inside",
      "focusin@inside(null) active=inside",
      "focusout@inside(null) active=inside",
    ]);
    expect(active()).toBe("inside");
  });

  test("the host's move: blur, then focus, each with the other node as relatedTarget", () => {
    const { rt, nodes, log, active } = nestedWorld();
    rt.host.dispatcher(nodes.inside.__vnId, rt.EV_CODES.focus, 0, 0, 0, 0, undefined);
    log.length = 0;
    rt.host.dispatcher(nodes.inside.__vnId, rt.EV_CODES.blur, nodes.trigger.__vnId, 0, 0, 0, undefined);
    rt.host.dispatcher(nodes.trigger.__vnId, rt.EV_CODES.focus, nodes.inside.__vnId, 0, 0, 0, undefined);
    expect(log).toEqual([
      "blur@inside(trigger) active=body",
      "focusout@inside(trigger) active=body",
      "focus@trigger(inside) active=trigger",
      "focusin@trigger(inside) active=trigger",
    ]);
    expect(active()).toBe("trigger");
  });

  test("the host's move: a focus handler that moves the focus on cancels the focusin", () => {
    const { rt, nodes, log, on, active } = nestedWorld();
    on("trigger", "focus", () => focus(nodes.other));
    rt.host.dispatcher(nodes.trigger.__vnId, rt.EV_CODES.focus, 0, 0, 0, 0, undefined);
    expect(log).toEqual([
      "focus@trigger(null) active=trigger",
      "focus@other(null) active=other",
      "focusin@other(null) active=other",
    ]);
    expect(active()).toBe("other");
  });
});

describe("location.reload()", () => {
  test("asks the host, which re-runs the app at the next frame", () => {
    const location: Record<string, unknown> = { href: "app://redjs/" };
    const rt = createRuntime({ globals: { location } });
    (location.reload as () => void)();
    expect(rt.host.reloads).toBe(1);
  });

  test("__VN_UNLOAD__: pagehide, the teardown, then the old app's timers, frames, sockets and listeners are gone", () => {
    const target = new EventTarget();
    const live = { timers: new Map<number, unknown>(), frames: new Map<number, unknown>(), next: 1 };
    const sockets: FakeSocket[] = [];
    class FakeSocket extends EventTarget {
      closed = false;
      onmessage: unknown = null;
      constructor(public url: string) {
        super();
        sockets.push(this);
      }
      close() {
        this.closed = true;
      }
    }
    const globals = {
      addEventListener: target.addEventListener.bind(target),
      removeEventListener: target.removeEventListener.bind(target),
      dispatchEvent: target.dispatchEvent.bind(target),
      setTimeout: (cb: unknown) => (live.timers.set(live.next, cb), live.next++),
      setInterval: (cb: unknown) => (live.timers.set(live.next, cb), live.next++),
      clearTimeout: (id: number) => live.timers.delete(id),
      clearInterval: (id: number) => live.timers.delete(id),
      requestAnimationFrame: (cb: unknown) => (live.frames.set(live.next, cb), live.next++),
      cancelAnimationFrame: (id: number) => live.frames.delete(id),
      WebSocket: FakeSocket,
    };
    const rt = createRuntime({ globals });
    const g = rt.global as unknown as Record<string, (...a: unknown[]) => unknown> & {
      __VN_UNLOAD__?: () => void;
      __VN_TEARDOWN__?: () => void;
      WebSocket: new (url: string) => FakeSocket;
    };
    const order: string[] = [];
    g.addEventListener("pagehide", () => order.push("pagehide"));
    let messages = 0;
    g.addEventListener("message", () => messages++);
    g.__VN_TEARDOWN__ = () => order.push("teardown");
    g.setTimeout(() => {});
    g.setInterval(() => {});
    g.requestAnimationFrame(() => {});
    const kept = g.setTimeout(() => {}) as number;
    g.clearTimeout(kept);
    const ws = new g.WebSocket("wss://example.invalid/");
    expect(ws).toBeInstanceOf(FakeSocket);
    let wsCalls = 0;
    ws.addEventListener("message", () => wsCalls++);
    ws.onmessage = () => wsCalls++;

    g.__VN_UNLOAD__!();

    expect(order).toEqual(["pagehide", "teardown"]);
    expect([live.timers.size, live.frames.size]).toEqual([0, 0]);
    expect(sockets.map((s) => s.closed)).toEqual([true]);
    ws.dispatchEvent(new Event("message"));
    expect([wsCalls, ws.onmessage]).toEqual([0, null]);
    target.dispatchEvent(new Event("message"));
    expect(messages).toBe(0);
    expect(g.__VN_UNLOAD__).toBeUndefined();
    expect(rt.advisories().some((l: string) => l.includes("SAME JavaScript realm"))).toBe(true);
  });
});


describe("native text field select()", () => {
  function field(tag = "input", type = "text") {
    const rt = createRuntime();
    const root = rt.nodeOps.createElement("div");
    const input = rt.nodeOps.createElement(tag);
    rt.nodeOps.insert(input, root, null);
    if (tag === "input") rt.patchProp(input, "type", null, type);
    rt.patchProp(input, "value", null, "Argon Привет 🙂\nSecond line");
    rt.host.focus.answer = () => "moved:0";
    return { rt, root, input };
  }

  test.each(["input", "textarea"])("%s.select() selects the native editor without manufacturing an edit", async tag => {
    const { rt, root, input } = field(tag);
    const seen: unknown[][] = [];
    input.addEventListener("input", () => seen.push(["input"]));
    input.addEventListener("change", () => seen.push(["change"]));
    root.addEventListener("select", (e: any) => { seen.push([e.type, e.target === input, e.bubbles, e.cancelable]); e.preventDefault(); });
    const before = input.value;
    expect(input.select()).toBeUndefined();
    expect(rt.host.inputSelectCalls).toEqual([input.__vnId]);
    expect(rt.global.document.activeElement).toBe(input);
    expect(input.value).toBe(before);
    expect(seen).toEqual([]);
    await new Promise<void>(resolve => queueMicrotask(resolve));
    expect(seen).toEqual([["select", true, true, false]]);
    expect(rt.advisories().every((note: string) => note.includes('addEventListener("change")'))).toBe(true);
  });

  test("nextTick select survives a native field whose first build/focus is pending", async () => {
    const { rt, input } = field("textarea");
    rt.host.focus.answer = () => "pending";
    const selected: string[] = [];
    rt.patchProp(input, "onSelect", null, () => selected.push(input.value));
    input.select();
    expect(rt.host.inputSelectCalls).toEqual([input.__vnId]);
    expect(input.value).toBe("Argon Привет 🙂\nSecond line");
    await new Promise<void>(resolve => queueMicrotask(resolve));
    expect(selected).toEqual([input.value]);
    expect(rt.advisories().every((line: string) => line.includes("element.focus()"))).toBe(true);
  });

  test("read-only and disabled text can be selected; only focus is refused when disabled", () => {
    const { rt, input } = field();
    rt.patchProp(input, "readonly", null, ""); input.select();
    rt.patchProp(input, "disabled", null, ""); input.select();
    expect(rt.host.inputSelectCalls).toEqual([input.__vnId, input.__vnId]);
    expect(rt.host.focus.calls).toHaveLength(1);
  });

  test("non-text controls never create a native text selection", () => {
    for (const type of ["file", "color", "range"]) {
      const { rt, input } = field("input", type);
      input.select();
      expect(rt.host.inputSelectCalls).toEqual([]);
    }
  });
});
