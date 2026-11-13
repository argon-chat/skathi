// `launchQueue` (tier 1) and `appLaunch` (tier 2): the JS half. The Rust half (argv parser, instance pipe, forward
// codec) is unit-tested in crates/vn-desktop; e2e/launch.spec drives a real second process.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type Params = { targetURL: string | null; files: readonly unknown[] };

describe("launchQueue", () => {
  test("this process's own launch is buffered until setConsumer, then delivered synchronously", () => {
    const rt = createRuntime({ launch: { url: "notes://open?id=7" } });
    const seen: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => seen.push(p));
    expect(seen.length).toBe(1);
    expect(seen[0].targetURL).toBe("notes://open?id=7");
    expect(seen[0].files).toEqual([]);
    expect(seen[0]).toBeInstanceOf(rt.global.LaunchParams);
  });

  test("a plain launch is one LaunchParams with a null targetURL", () => {
    const rt = createRuntime();
    const seen: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => seen.push(p));
    expect(seen.map((p) => p.targetURL)).toEqual([null]);
    expect(rt.advisories()).toEqual([]);
  });

  test("a forwarded launch goes straight to the consumer; a replaced consumer gets only later ones", () => {
    const rt = createRuntime();
    const first: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => first.push(p));
    rt.host.desktop.forward({ argv: ["x.exe", "notes://a"], cwd: "C:\\", url: "notes://a", paths: [] });
    expect(first.map((p) => p.targetURL)).toEqual([null, "notes://a"]);
    const second: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => second.push(p));
    expect(second).toEqual([]);
    rt.host.desktop.forward({ argv: [], cwd: "C:\\", url: null, paths: [] });
    expect(second.length).toBe(1);
    expect(first.length).toBe(2);
  });

  test("launches that arrive before any consumer queue up in order", () => {
    const rt = createRuntime();
    rt.host.desktop.forward({ argv: [], cwd: "C:\\", url: "notes://1", paths: [] });
    rt.host.desktop.forward({ argv: [], cwd: "C:\\", url: "notes://2", paths: [] });
    const seen: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => seen.push(p));
    expect(seen.map((p) => p.targetURL)).toEqual([null, "notes://1", "notes://2"]);
  });

  test("files are handles for the launch's paths, holding no path", async () => {
    const rt = createRuntime({
      launch: { paths: ["C:\\docs\\a.txt"], handles: [{ h: 7, name: "a.txt", kind: "file" }] },
    });
    const seen: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => seen.push(p));
    rt.host.desktop.forward({
      argv: [],
      cwd: "C:\\",
      url: null,
      paths: ["C:\\docs\\proj"],
      handles: [{ h: 8, name: "proj", kind: "directory" }],
    });
    expect(seen.length).toBe(2);
    const [a] = seen[0].files as { kind: string; name: string }[];
    const [b] = seen[1].files as { kind: string; name: string }[];
    expect([a.name, a.kind, b.name, b.kind]).toEqual(["a.txt", "file", "proj", "directory"]);
    expect(a).toBeInstanceOf(rt.global.FileSystemFileHandle);
    expect(b).toBeInstanceOf(rt.global.FileSystemDirectoryHandle);
    expect(JSON.stringify(a)).not.toContain("docs");
    // The handle answers through the host by id, not by path.
    rt.host.desktop.fs.answers.set("getFile", (h: number) => ({ payload: { name: "a.txt", size: 1 }, value: { h } }));
    expect(await (a as unknown as { getFile(): Promise<unknown> }).getFile()).toEqual({ h: 7 });
    expect(rt.advisories().some((l: string) => l.includes("LaunchParams.files"))).toBe(false);
  });

  test("a consumer that throws does not take the next launch with it", () => {
    const rt = createRuntime();
    let n = 0;
    rt.global.launchQueue.setConsumer(() => {
      n++;
      throw new Error("boom");
    });
    rt.host.desktop.forward({ argv: [], cwd: "C:\\", url: "notes://x", paths: [] });
    expect(n).toBe(2);
    expect(rt.host.logs.some((l: string) => l.includes("launchQueue consumer threw"))).toBe(true);
  });

  test("setConsumer with a non-function is the browser's TypeError", () => {
    const rt = createRuntime();
    expect(() => rt.global.launchQueue.setConsumer(42)).toThrow(TypeError);
  });
});

describe("appLaunch", () => {
  test("reads this launch: argv, cwd, url, paths, singleInstance", () => {
    const rt = createRuntime({
      launch: {
        argv: ["C:\\apps\\Notes.exe", "a.txt", "--flag"],
        cwd: "C:\\docs",
        url: null,
        paths: ["C:\\docs\\a.txt"],
        singleInstance: true,
      },
    });
    expect(rt.appLaunch.argv).toEqual(["C:\\apps\\Notes.exe", "a.txt", "--flag"]);
    expect(rt.appLaunch.cwd).toBe("C:\\docs");
    expect(rt.appLaunch.url).toBe(null);
    expect(rt.appLaunch.paths).toEqual(["C:\\docs\\a.txt"]);
    expect(rt.appLaunch.singleInstance).toBe(true);
    expect(Object.isFrozen(rt.appLaunch.argv)).toBe(true);
  });

  test("a forwarded launch fires `launch` with the second process's argv, cwd, url, paths and source", () => {
    const rt = createRuntime();
    const events: Record<string, unknown>[] = [];
    rt.appLaunch.addEventListener("launch", (e: Record<string, unknown>) =>
      events.push({ argv: e.argv, cwd: e.cwd, url: e.url, paths: e.paths, source: e.source, type: e.type }),
    );
    rt.host.desktop.forward({
      argv: ["C:\\apps\\Notes.exe", "b.txt"],
      cwd: "D:\\work",
      url: null,
      paths: ["D:\\work\\b.txt"],
    });
    expect(events).toEqual([
      {
        argv: ["C:\\apps\\Notes.exe", "b.txt"],
        cwd: "D:\\work",
        url: null,
        paths: ["D:\\work\\b.txt"],
        source: "second-instance",
        type: "launch",
      },
    ]);
  });

  test("the tier-2 event fires before the tier-1 queue delivers", () => {
    const rt = createRuntime();
    const order: string[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => order.push(`queue ${p.targetURL}`));
    rt.appLaunch.addEventListener("launch", () => {
      order.push("event");
    });
    rt.host.desktop.forward({ argv: [], cwd: "C:\\", url: "notes://z", paths: [] });
    expect(order).toEqual(["queue null", "event", "queue notes://z"]);
  });

  test("registerProtocol is sync, throws a TypeError for bad or foreign schemes; the other two answer booleans", () => {
    const rt = createRuntime();
    rt.host.desktop.protocols.set("theirs", "gl.argon.other");
    expect(rt.appLaunch.isProtocolRegistered("notes")).toBe(false);
    expect(rt.appLaunch.registerProtocol("Notes")).toBe(undefined);
    expect(rt.appLaunch.isProtocolRegistered("notes")).toBe(true);
    expect(() => rt.appLaunch.registerProtocol("https")).toThrow(TypeError);
    expect(() => rt.appLaunch.registerProtocol("ms-settings")).toThrow(TypeError);
    expect(() => rt.appLaunch.registerProtocol("theirs")).toThrow(/gl\.argon\.other/);
    expect(rt.appLaunch.unregisterProtocol("theirs")).toBe(false);
    expect(rt.appLaunch.unregisterProtocol("notes")).toBe(true);
    expect(rt.appLaunch.unregisterProtocol("notes")).toBe(false);
  });

  test("with no desktop host: an empty launch, and no LaunchParams at all", () => {
    const rt = createRuntime({ desktop: false });
    expect(rt.appLaunch.argv).toEqual([]);
    expect(rt.appLaunch.singleInstance).toBe(false);
    const seen: Params[] = [];
    rt.global.launchQueue.setConsumer((p: Params) => seen.push(p));
    expect(seen).toEqual([]);
    expect(() => rt.appLaunch.registerProtocol("notes")).toThrow(TypeError);
    expect(rt.appLaunch.isProtocolRegistered("notes")).toBe(false);
  });
});
