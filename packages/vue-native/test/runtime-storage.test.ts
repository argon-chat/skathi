// The `appStorage` extension tier: the JSON unwrapping, what throws instead of returning a path, and the inert
// fallback. The standard tier (`localStorage`) is pinned by Rust tests in redjs and crates/vn-storage.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

describe("appStorage", () => {
  test("resolves a known folder and joins a subfolder", () => {
    const rt = createRuntime();
    const folder = rt.appStorage.getUserFolder("APPDATA", "myAppFolder");
    expect(folder).toBe("C:\\Users\\test\\AppData\\Roaming\\myAppFolder");
    expect(rt.appStorage.setStorageFolder(folder)).toBe(folder);
    expect(rt.host.storage.setCalls).toEqual([{ path: folder, encrypted: "keep" }]);
  });

  test("the subfolder is optional", () => {
    const rt = createRuntime();
    expect(rt.appStorage.getUserFolder("LOCALAPPDATA")).toBe("C:\\Users\\test\\AppData\\Local");
    expect(rt.host.storage.folderCalls).toEqual([{ kind: "LOCALAPPDATA", sub: "" }]);
  });

  test("every kind the host knows is reachable", () => {
    const rt = createRuntime();
    expect(rt.appStorage.kinds).toEqual(["APPDATA", "LOCALAPPDATA", "TEMP", "DOCUMENTS", "HOME"]);
    for (const kind of rt.appStorage.kinds) {
      expect(rt.appStorage.getUserFolder(kind)).toStartWith("C:\\Users\\test");
    }
  });

  test("an unknown kind throws instead of returning a path nobody checked", () => {
    // Prevents `getUserFolder("APP_DATA")` returning "" and the app writing to the working directory.
    const rt = createRuntime();
    expect(() => rt.appStorage.getUserFolder("APP_DATA")).toThrow(TypeError);
    expect(() => rt.appStorage.getUserFolder("APP_DATA")).toThrow(/not a user folder kind/);
  });

  test("a subfolder that climbs out is refused by the host and rethrown here", () => {
    const rt = createRuntime();
    expect(() => rt.appStorage.getUserFolder("APPDATA", "../../Windows")).toThrow(TypeError);
  });

  test("setStorageFolder passes the encryption flag through and reports it back", () => {
    const rt = createRuntime();
    rt.appStorage.setStorageFolder("C:\\data", { encrypted: true });
    expect(rt.host.storage.setCalls).toEqual([{ path: "C:\\data", encrypted: "true" }]);
    expect(rt.appStorage.encrypted).toBe(true);
    expect(rt.appStorage.folder).toBe("C:\\data");
    // A missing options object and `{}` KEEP the format: an encrypted folder must not come out of a move in plaintext.
    rt.appStorage.setStorageFolder("C:\\moved");
    rt.appStorage.setStorageFolder("C:\\moved", {});
    expect(rt.host.storage.setCalls.slice(1).map((c) => c.encrypted)).toEqual(["keep", "keep"]);
    expect(rt.appStorage.encrypted).toBe(true);
    rt.appStorage.setStorageFolder("C:\\moved", { encrypted: false });
    expect(rt.host.storage.setCalls[3]?.encrypted).toBe("false");
    expect(rt.appStorage.encrypted).toBe(false);
  });

  test("a refused setStorageFolder throws rather than silently keeping the old one", () => {
    const rt = createRuntime();
    rt.host.storage.refuse = "encrypted storage is not available on this platform";
    expect(() => rt.appStorage.setStorageFolder("C:\\data", { encrypted: true })).toThrow(
      /not available on this platform/,
    );
  });

  test("info() is one call and carries the last error", () => {
    const rt = createRuntime();
    rt.host.storage.bytes = 512;
    rt.host.storage.error = "could not write C:\\data\\localStorage.json: access denied";
    const info = rt.appStorage.info();
    expect(info.folder).toBe("C:\\Users\\test\\AppData\\Local\\hello\\storage");
    expect(info.file).toEndWith("localStorage.json");
    expect(info.bytes).toBe(512);
    expect(info.provider).toBe("Windows DPAPI (per-user)");
    expect(info.error).toMatch(/access denied/);
  });

  test("flush() reports whether there was anything to write", () => {
    const rt = createRuntime();
    expect(rt.appStorage.flush()).toBe(false);
    rt.host.storage.pending = true;
    expect(rt.appStorage.flush()).toBe(true);
    expect(rt.appStorage.flush()).toBe(false);
    expect(rt.host.storage.flushes).toBe(3);
  });

  test("a failing flush throws — a lost write must not read as a successful one", () => {
    const rt = createRuntime();
    rt.host.storage.flushError = "could not write C:\\data\\localStorage.json: access denied";
    expect(() => rt.appStorage.flush()).toThrow(/access denied/);
  });

  test("arguments are coerced the way every other host-fn wrapper coerces them", () => {
    const rt = createRuntime();
    expect(() => rt.appStorage.getUserFolder(null as unknown as string)).toThrow(TypeError);
    expect(rt.host.storage.folderCalls.at(-1)).toEqual({ kind: "null", sub: "" });
  });
});

describe("appStorage without a host", () => {
  test("the two path verbs throw rather than answering with a plausible path", () => {
    const rt = createRuntime({ storage: false });
    expect(() => rt.appStorage.getUserFolder("APPDATA")).toThrow(/no storage host/);
    expect(() => rt.appStorage.setStorageFolder("C:\\data")).toThrow(/no storage host/);
  });

  test("the read-only members answer emptily, so a settings screen still renders", () => {
    const rt = createRuntime({ storage: false });
    expect(rt.appStorage.kinds).toEqual([]);
    expect(rt.appStorage.folder).toBe("");
    expect(rt.appStorage.encrypted).toBe(false);
    expect(rt.appStorage.flush()).toBe(false);
    expect(rt.appStorage.info().error).toBe("no storage host");
  });

  test("it advises exactly once, naming the feature", () => {
    const rt = createRuntime({ storage: false });
    rt.appStorage.kinds;
    rt.appStorage.folder;
    rt.appStorage.flush();
    expect(rt.advisories()).toHaveLength(1);
    expect(rt.advisories()[0]).toMatch(/storages/);
  });

  test("a real host does NOT advise", () => {
    const rt = createRuntime();
    rt.appStorage.getUserFolder("APPDATA");
    rt.appStorage.info();
    expect(rt.advisories()).toEqual([]);
  });
});
