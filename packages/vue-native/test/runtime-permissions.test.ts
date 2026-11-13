// The `appPermissions` extension tier: the four-state read, the promise plumbing behind `request()`, the advisory and
// the `change` re-broadcast. The standard tier (`navigator.permissions`) is pinned by Rust tests in redjs.

import { describe, expect, test } from "bun:test";
import { createRuntime, PERMISSION_NAMES } from "./runtime-harness";

type ChangeEvent = VnEvent & { name: string; state: string };

function withStates(states: Record<string, string>) {
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
    // The two lists are the same contract written twice (`Capability::ALL` and runtime.js).
    const rt = createRuntime();
    expect(rt.appPermissions.names).toEqual(PERMISSION_NAMES);
  });

  test("query is SYNCHRONOUS and reports all four states", () => {
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
    expect(byName.microphone).toEqual({
      name: "microphone",
      state: "granted",
      canRequest: false,
      deniedBySystem: false,
    });
    expect(byName.camera).toEqual({ name: "camera", state: "denied", canRequest: true, deniedBySystem: false });
    expect(byName.geolocation?.canRequest).toBe(false);
  });

  test("request() resolves immediately when the host settled synchronously", async () => {
    const rt = withStates({ microphone: "granted" });
    await expect(rt.appPermissions.request("microphone")).resolves.toBe("granted");
    expect(rt.host.permissions.requests).toEqual(["microphone"]);
  });

  test("a pending request settles through the pump, with its token", async () => {
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
    // A stale token can arrive after an HMR reload swapped the JS world; the dispatcher must survive it.
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
    // ONE native change subscription, owned by `navigator.permissions`; the extension listens to it.
    const rt = withStates({ microphone: "granted" });
    await Promise.resolve();
    await Promise.resolve();

    const seen: string[][] = [];
    rt.appPermissions.addEventListener("change", (e) => {
      const change = e as ChangeEvent;
      seen.push([change.name, change.state]);
    });

    rt.host.permissions.move("microphone", "denied");
    expect(seen).toEqual([["microphone", "denied"]]);

    rt.host.permissions.move("camera", "granted");
    expect(seen).toEqual([
      ["microphone", "denied"],
      ["camera", "granted"],
    ]);
  });

  test("the change payload carries the FOUR-state value, not the web projection", async () => {
    const rt = withStates({ notifications: "granted" });
    await Promise.resolve();
    await Promise.resolve();
    const seen: string[] = [];
    rt.appPermissions.addEventListener("change", (e) => {
      seen.push((e as ChangeEvent).state);
    });
    rt.host.permissions.move("notifications", "unsupported");
    expect(seen).toEqual(["unsupported"]);
  });
});
