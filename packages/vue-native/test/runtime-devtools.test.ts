// The `appDevtools` extension tier (docs/DEVTOOLS.md "Turning them off").
//
// What is NOT tested here is the half that matters most and cannot be reached
// from a fake host: that `disable()` really closes a TCP listener, deletes the
// handshake and makes the port refuse. That is an END-TO-END claim about a real
// process and it is pinned by `packages/vn-test/specs/devtools-optout.spec.ts`,
// which connects to the port before and after. What lives in JS — the JSON
// round trip, the refusal advisory, the `onChange` fan-out and the inert branch
// a `--features no-devtools` host produces — is here.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

describe("appDevtools", () => {
  test("the default is ON, which is the whole point of the m22 inversion", () => {
    // Before m22 a shipped binary had no devtools unless it opted in at COMPILE
    // time. Now every build has them and every build starts with them on; the
    // app is what turns them off. `available` is the other half of that answer:
    // "is there anything here to switch".
    const rt = createRuntime();
    expect(rt.appDevtools.available).toBe(true);
    expect(rt.appDevtools.isEnabled()).toBe(true);
    expect(rt.appDevtools.info().port).toBe(49221);
  });

  test("disable() turns them off and reports the state afterwards", () => {
    const rt = createRuntime();
    expect(rt.appDevtools.disable()).toBe(false);
    expect(rt.host.devtools.setCalls).toEqual([false]);
    expect(rt.appDevtools.isEnabled()).toBe(false);
    // The port going to null is how an app can tell the LISTENER is gone rather
    // than just the key being refused — the distinction this API exists for.
    expect(rt.appDevtools.info().port).toBeNull();
    expect(rt.advisories()).toEqual([]);
  });

  test("enable() turns them back on, and both verbs are idempotent", () => {
    const rt = createRuntime();
    rt.appDevtools.disable();
    expect(rt.appDevtools.enable()).toBe(true);
    expect(rt.appDevtools.info().port).toBe(49221);
    // Calling either again is a no-op that still answers truthfully: an app
    // that calls `enable()` on every route change must not be surprising.
    expect(rt.appDevtools.enable()).toBe(true);
    expect(rt.appDevtools.disable()).toBe(false);
    expect(rt.appDevtools.disable()).toBe(false);
  });

  test("VN_DEVTOOLS=1 refuses disable() — with an advisory, not a throw", () => {
    // The developer-override case: somebody is debugging a build whose app
    // turns its devtools off. The app's call must not silently appear to work
    // (that is the "diverge without saying so" failure this project refuses),
    // and it must not take a shipped app's boot code down either.
    const rt = createRuntime();
    rt.host.devtools.locked = true;
    expect(rt.appDevtools.disable()).toBe(true);
    expect(rt.appDevtools.isEnabled()).toBe(true);
    expect(rt.advisories()).toHaveLength(1);
    expect(rt.advisories()[0]).toMatch(/disable\(\) was refused/);
    expect(rt.advisories()[0]).toMatch(/VN_DEVTOOLS=1/);
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
    // The second listener still ran, and the throw is reported once.
    expect(seen).toEqual([false]);
    expect(rt.advisories().some((a: string) => /change listener threw/.test(a))).toBe(true);
  });

  test("open() is programmatic F12 and reports what the host did", () => {
    const rt = createRuntime();
    expect(rt.appDevtools.open()).toBe(true);
    expect(rt.host.devtools.opens).toBe(1);
    expect(rt.appDevtools.info().inspectors).toBe(1);

    rt.host.devtools.openError = "the devtools are disabled in this process";
    expect(rt.appDevtools.open()).toBe(false);
    expect(rt.advisories().some((a: string) => /open\(\) did not open an inspector/.test(a))).toBe(
      true,
    );
  });

  test("info() is the host's own object, field for field", () => {
    // The shape is a contract with `devtools::state_json`; a field that
    // disappears there and lingers here is exactly the drift this pins.
    const rt = createRuntime();
    expect(Object.keys(rt.appDevtools.info()).sort()).toEqual([
      "enabled",
      "inspectors",
      "key",
      "locked",
      "pid",
      "port",
      "slot",
    ]);
  });

  test("`--features no-devtools`: inert, honest, and it does not throw", () => {
    // The build that has no devtools AT ALL. `appDevtools.disable()` in an
    // entry module is its likeliest caller, and a build that already has none
    // must not crash on being asked to turn them off — so `disable()` is a
    // silent success and only the two verbs that ASK FOR SOMETHING advise.
    const rt = createRuntime({ devtools: false });
    expect(rt.appDevtools.available).toBe(false);
    expect(rt.appDevtools.isEnabled()).toBe(false);
    expect(rt.appDevtools.disable()).toBe(false);
    expect(rt.advisories()).toEqual([]);

    expect(rt.appDevtools.enable()).toBe(false);
    expect(rt.advisories()).toHaveLength(1);
    expect(rt.advisories()[0]).toMatch(/no-devtools/);

    expect(rt.appDevtools.open()).toBe(false);
    expect(rt.appDevtools.info()).toEqual({
      enabled: false,
      locked: false,
      port: null,
      pid: 0,
      key: false,
      inspectors: 0,
      slot: "",
    });
    // The registry is still real, so no caller special-cases this build.
    const off = rt.appDevtools.onChange(() => {});
    expect(typeof off).toBe("function");
    off();
  });
});
