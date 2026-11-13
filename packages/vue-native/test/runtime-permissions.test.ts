// The `appPermissions` extension tier.
//
// Not tested here: the standard tier (`navigator.permissions` is redjs's, pinned by Rust tests in
// vendor/redjs/redjs/src/webapi/permissions.rs). Here: the four-state read, the promise plumbing behind `request()`,
// the advisory, and the `change` re-broadcast.

import { describe, expect, test } from "bun:test";
import { createRuntime, PERMISSION_NAMES } from "./runtime-harness";

/** A runtime whose backend answers from `states`. */
function withStates(states) {
  const rt = createRuntime();
  for (const [name, state] of Object.entries(states)) rt.host.permissions.states.set(name, state);
  return rt;
}

describe("appPermissions", () => {
  test("names the platform, so an advisory can say where it is standing", () => {
    const rt = createRuntime();
    expect(rt.appPermissions.platform).toBe("TestOS");
  });

  test("knows exactly the W3C names the Rust side knows", () => {
    // The two lists are the same contract written twice — one in
    // `Capability::ALL`, one in runtime.js. A name in one and not the other is
    // a permission that either cannot be queried or cannot be requested.
    const rt = createRuntime();
    expect(rt.appPermissions.names).toEqual(PERMISSION_NAMES);
  });

  test("query is SYNCHRONOUS and reports all four states", () => {
    // The shape difference from `navigator.permissions.query()` is deliberate:
    // a promise and a plain string are hard to confuse at a call site.
    const rt = withStates({
      microphone: "granted",
      camera: "denied",
      "display-capture": "prompt",
      geolocation: "unsupported",
    });
    expect(rt.appPermissions.query("microphone")).toBe("granted");
    expect(rt.appPermissions.query("camera")).toBe("denied");
    expect(rt.appPermissions.query("display-capture")).toBe("prompt");
    expect(rt.appPermissions.query("geolocation")).toBe("unsupported");
  });

  test("an unknown name throws instead of quietly reading as denied", () => {
    const rt = createRuntime();
    expect(() => rt.appPermissions.query("mic")).toThrow(TypeError);
  });

  test("'unsupported' advises once, naming the platform and the capability", () => {
    // Compat-first: never a silent divergence. And exactly once — an advisory
    // per poll would drown the log it is meant to be findable in.
    const rt = withStates({ notifications: "unsupported" });
    rt.appPermissions.query("notifications");
    rt.appPermissions.query("notifications");
    rt.appPermissions.query("notifications");
    const advisories = rt.host.logs.filter((l) => l.startsWith("[vue-native compat]"));
    expect(advisories).toHaveLength(1);
    expect(advisories[0]).toContain("notifications");
    expect(advisories[0]).toContain("TestOS");
    expect(advisories[0]).toContain("request() is a no-op");
  });

  test("a granted capability never advises", () => {
    const rt = withStates({ microphone: "granted" });
    rt.appPermissions.query("microphone");
    expect(rt.host.logs.filter((l) => l.startsWith("[vue-native compat]"))).toHaveLength(0);
  });

  test("list() returns every capability with its state and whether asking would help", () => {
    const rt = withStates({ microphone: "granted", camera: "denied" });
    const rows = rt.appPermissions.list();
    expect(rows).toHaveLength(PERMISSION_NAMES.length);
    const byName = Object.fromEntries(rows.map((r) => [r.name, r]));
    expect(byName.microphone).toEqual({ name: "microphone", state: "granted", canRequest: false, deniedBySystem: false });
    expect(byName.camera).toEqual({ name: "camera", state: "denied", canRequest: true, deniedBySystem: false });
    // Nothing to ask when the platform has no notion of it.
    expect(byName.geolocation.canRequest).toBe(false);
  });

  test("request() resolves immediately when the host settled synchronously", async () => {
    const rt = withStates({ microphone: "granted" });
    await expect(rt.appPermissions.request("microphone")).resolves.toBe("granted");
    expect(rt.host.permissions.requests).toEqual(["microphone"]);
  });

  test("a pending request settles through the pump, with its token", async () => {
    // The whole reason `request()` is a promise: on Windows this is the user
    // walking to Settings and coming back, delivered by `vn_permissions::pump`.
    const rt = withStates({ camera: "denied" });
    rt.host.permissions.pendingToken = 7;
    const p = rt.appPermissions.request("camera");
    let settled = false;
    p.then(() => {
      settled = true;
    });
    await Promise.resolve();
    expect(settled).toBe(false);

    rt.host.permissions.settle(7, "granted");
    await expect(p).resolves.toBe("granted");
  });

  test("a settlement for an unknown token is ignored rather than throwing", async () => {
    // A stale token can arrive after an HMR reload swapped the JS world; the
    // dispatcher must survive it.
    const rt = withStates({ camera: "denied" });
    expect(() => rt.host.permissions.settle(999, "granted")).not.toThrow();
  });

  test("two concurrent requests settle independently", async () => {
    const rt = withStates({ camera: "denied", microphone: "denied" });
    rt.host.permissions.pendingToken = 1;
    const a = rt.appPermissions.request("camera");
    rt.host.permissions.pendingToken = 2;
    const b = rt.appPermissions.request("microphone");

    rt.host.permissions.settle(2, "granted");
    rt.host.permissions.settle(1, "denied");
    expect(await a).toBe("denied");
    expect(await b).toBe("granted");
  });

  test("request() rejects a name that is not a permission", async () => {
    const rt = createRuntime();
    await expect(rt.appPermissions.request("nope")).rejects.toThrow(TypeError);
  });

  test("change is re-broadcast from the standard PermissionStatus objects", async () => {
    // The layering that matters: there is ONE native change subscription, owned
    // by `navigator.permissions`, and the extension listens to it rather than
    // opening a second one that would race the first.
    const rt = withStates({ microphone: "granted" });
    // The subscription is set up from a `.then`, so let it land.
    await Promise.resolve();
    await Promise.resolve();

    const seen = [];
    rt.appPermissions.addEventListener("change", (e) => seen.push([e.name, e.state]));

    rt.host.permissions.move("microphone", "denied");
    expect(seen).toEqual([["microphone", "denied"]]);

    rt.host.permissions.move("camera", "granted");
    expect(seen).toEqual([
      ["microphone", "denied"],
      ["camera", "granted"],
    ]);
  });

  test("the change payload carries the FOUR-state value, not the web projection", async () => {
    // `navigator.permissions` has to say "denied" for a capability the platform
    // does not have; the extension exists so an app can tell the two apart, and
    // that has to hold on the event as well as on `query()`.
    const rt = withStates({ notifications: "granted" });
    await Promise.resolve();
    await Promise.resolve();
    const seen = [];
    rt.appPermissions.addEventListener("change", (e) => seen.push(e.state));
    rt.host.permissions.move("notifications", "unsupported");
    expect(seen).toEqual(["unsupported"]);
  });
});
