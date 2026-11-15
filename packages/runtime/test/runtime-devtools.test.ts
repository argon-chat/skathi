// The `appDevtools` extension tier: the JSON round trip, the refusal advisory, the `onChange` fan-out and the inert branch
// of a `--features no-devtools` host. That `disable()` really closes the listener is end-to-end (sk-test devtools-optout.spec.ts).

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

describe("appDevtools", () => {
  test("disable() turns them off and reports the state afterwards", () => {
    const rt = createRuntime();
    expect(rt.appDevtools.disable()).toBe(false);
    expect(rt.host.devtools.setCalls).toEqual([false]);
    expect(rt.appDevtools.isEnabled()).toBe(false);
    expect(rt.appDevtools.info().port).toBeNull();
    expect(rt.advisories()).toEqual([]);
  });

  test("enable() turns them back on, and both verbs are idempotent", () => {
    const rt = createRuntime();
    rt.appDevtools.disable();
    expect(rt.appDevtools.enable()).toBe(true);
    expect(rt.appDevtools.info().port).toBe(49221);
    expect(rt.appDevtools.enable()).toBe(true);
    expect(rt.appDevtools.disable()).toBe(false);
    expect(rt.appDevtools.disable()).toBe(false);
  });

  test("SK_DEVTOOLS=1 refuses disable() — with an advisory, not a throw", () => {
    const rt = createRuntime();
    rt.host.devtools.locked = true;
    expect(rt.appDevtools.disable()).toBe(true);
    expect(rt.appDevtools.isEnabled()).toBe(true);
    expect(rt.advisories()).toHaveLength(1);
    expect(rt.appDevtools.info().locked).toBe(true);
  });

  test("onChange fires on settled changes only, and unsubscribes", () => {
    const rt = createRuntime();
    const seen: boolean[] = [];
    const off = rt.appDevtools.onChange((on: boolean) => seen.push(on));

    rt.appDevtools.disable();
    rt.appDevtools.disable(); // no change, no event
    rt.appDevtools.enable();
    expect(seen).toEqual([false, true]);

    off();
    rt.appDevtools.disable();
    expect(seen).toEqual([false, true]);
  });

  test("a refused change is not a change, so no listener is told about it", () => {
    const rt = createRuntime();
    rt.host.devtools.locked = true;
    const seen: boolean[] = [];
    rt.appDevtools.onChange((on: boolean) => seen.push(on));
    rt.appDevtools.disable();
    expect(seen).toEqual([]);
  });

  test("a throwing listener is contained rather than failing the change", () => {
    const rt = createRuntime();
    const seen: boolean[] = [];
    rt.appDevtools.onChange(() => {
      throw new Error("boom");
    });
    rt.appDevtools.onChange((on: boolean) => seen.push(on));
    expect(rt.appDevtools.disable()).toBe(false);
    expect(seen).toEqual([false]);
  });

  test("open() is programmatic F12 and reports what the host did", () => {
    const rt = createRuntime();
    expect(rt.appDevtools.open()).toBe(true);
    expect(rt.host.devtools.opens).toBe(1);
    expect(rt.appDevtools.info().inspectors).toBe(1);

    rt.host.devtools.openError = "the devtools are disabled in this process";
    expect(rt.appDevtools.open()).toBe(false);
  });

  test("`--features no-devtools`: inert, honest, and it does not throw", () => {
    const rt = createRuntime({ devtools: false });
    expect(rt.appDevtools.available).toBe(false);
    expect(rt.appDevtools.isEnabled()).toBe(false);
    expect(rt.appDevtools.disable()).toBe(false);
    expect(rt.advisories()).toEqual([]);

    expect(rt.appDevtools.enable()).toBe(false);
    expect(rt.advisories()).toHaveLength(1);

    expect(rt.appDevtools.open()).toBe(false);
    const off = rt.appDevtools.onChange(() => {});
    expect(typeof off).toBe("function");
    off();
  });
});
