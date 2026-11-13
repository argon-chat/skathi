// The `appStorage` extension tier.
//
// Not tested here: the standard tier (`localStorage` / `sessionStorage` are redjs's, pinned by Rust tests in
// vendor/redjs/redjs/src/webapi/storage.rs, and the bytes on disk by crates/vn-storage). Here: the JSON unwrapping,
// what throws instead of returning a path, and the inert fallback.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

describe("appStorage", () => {
  test("resolves a known folder and joins a subfolder", () => {
    // The owner's sketch, executed:
    //   app.setStorageFolder(app.getUserFolder("APPDATA", "myAppFolder"))
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
    // The two lists are one contract written twice — `KINDS` in folders.rs and
    // whatever an app types. A kind in one and not the other is a folder no
    // app can name.
    const rt = createRuntime();
    expect(rt.appStorage.kinds).toEqual([
      "APPDATA",
      "LOCALAPPDATA",
      "TEMP",
      "DOCUMENTS",
      "HOME",
    ]);
    for (const kind of rt.appStorage.kinds) {
      expect(rt.appStorage.getUserFolder(kind)).toStartWith("C:\\Users\\test");
    }
  });

  test("an unknown kind throws instead of returning a path nobody checked", () => {
    // The failure this prevents: `getUserFolder("APP_DATA")` returning "" and
    // the app then writing its data to the process working directory.
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
    // A missing options object and `{}` say nothing: the format is KEPT —
    // an encrypted folder must not come out of a move in plaintext.
    rt.appStorage.setStorageFolder("C:\\moved");
    rt.appStorage.setStorageFolder("C:\\moved", {});
    expect(rt.host.storage.setCalls.slice(1).map((c) => c.encrypted)).toEqual(["keep", "keep"]);
    expect(rt.appStorage.encrypted).toBe(true);
    // Only an explicit `{ encrypted: false }` is "no".
    rt.appStorage.setStorageFolder("C:\\moved", { encrypted: false });
    expect(rt.host.storage.setCalls[3].encrypted).toBe("false");
    expect(rt.appStorage.encrypted).toBe(false);
  });

  test("a refused setStorageFolder throws rather than silently keeping the old one", () => {
    // The case that matters: asking for encryption on a platform with no
    // provider. Writing plaintext there would be the worst available outcome.
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
    expect(() => rt.appStorage.getUserFolder(null)).toThrow(TypeError);
    expect(rt.host.storage.folderCalls.at(-1)).toEqual({ kind: "null", sub: "" });
  });
});

describe("appStorage without a host", () => {
  // A host built `--no-default-features`, or an older one. The rule is the same
  // as `appWindow`/`appPermissions`: advise once, never pretend.

  test("the two path verbs throw rather than answering with a plausible path", () => {
    // The whole reason this branch is not a silent no-op: an app that gets ""
    // back from `getUserFolder` and passes it on writes its data SOMEWHERE
    // real, and finding out where afterwards is a support ticket.
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
    // Compat-first: never a silent divergence. And exactly once — one advisory
    // per property read would drown the log it is meant to be findable in.
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
