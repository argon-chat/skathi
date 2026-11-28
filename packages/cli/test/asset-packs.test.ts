// sk.config.json `packs`: assets dealt out to <name>.skpk files beside the
// payload, whose META names each pack with the SHA-256 of its index (header,
// TOC, names) — the id crates/sk-app/src/payload.rs checks when it opens one.

import { afterAll, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { splitPacks, writeIcuPack } from "../cli/build-app";
import { ConfigError, loadAppConfig } from "../cli/config";
import { readPayload } from "../cli/payload";
import { KIND, NAMES, PAGE } from "../cli/skpk";

const root = mkdtempSync(join(tmpdir(), "sk-asset-packs-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

function app(dir: string, config: Record<string, unknown>) {
  const at = join(root, dir);
  mkdirSync(join(at, "src"), { recursive: true });
  mkdirSync(join(at, "assets", "fonts"), { recursive: true });
  writeFileSync(join(at, "src", "main.ts"), "globalThis.x = 1;");
  writeFileSync(join(at, "assets", "fonts", "a.woff2"), "font a");
  writeFileSync(join(at, "assets", "fonts", "b.ttf"), "font b");
  writeFileSync(join(at, "assets", "logo.png"), "png");
  writeFileSync(join(at, "assets", "beep.wav"), "wav");
  writeFileSync(join(at, "sk.config.json"), JSON.stringify({ name: dir, version: "1.0.0", ...config }));
  return at;
}

describe("config", () => {
  test("packs are read in order, a missing match takes the rest", () => {
    const at = app("cfg", { packs: [{ name: "fonts", match: ["**/*.woff2", "fonts/**"] }, { name: "assets" }] });
    expect(loadAppConfig(at).packs).toEqual([
      { name: "fonts", match: ["**/*.woff2", "fonts/**"] },
      { name: "assets", match: null },
    ]);
    expect(loadAppConfig(app("none", {})).packs).toEqual([]);
  });

  test("a bad pack is an error", () => {
    for (const packs of [
      {},
      [{ name: "app" }],
      [{ name: "icu" }],
      [{ name: "Fonts" }],
      [{ name: "../x" }],
      [{ name: "a" }, { name: "a" }],
      [{ name: "a", match: [] }],
      [{ name: "a", match: "x" }],
      [{ name: "a", typo: 1 }],
    ]) {
      expect(() => loadAppConfig(app("bad", { packs }))).toThrow(ConfigError);
    }
  });
});

test('"permissions.ffi" keeps the path of the platform and refuses one that leaves the app', () => {
  const ffi = [
    { name: "db", path: "$APP/lib/db" },
    { name: "crt", path: { windows: "$SYSTEM/msvcrt", linux: "$SYSTEM/libc.so.6" } },
  ];
  const on = (platform: "windows" | "macos") =>
    loadAppConfig(app(`ffi-${platform}`, { permissions: { ffi } }), { platform }).permissions;
  expect(on("windows")).toEqual({
    ffi: [
      { name: "db", path: "$APP/lib/db" },
      { name: "crt", path: "$SYSTEM/msvcrt" },
    ],
    ffiAddress: false,
    process: [],
  });
  expect(on("macos")?.ffi).toEqual([{ name: "db", path: "$APP/lib/db" }]);
  expect(loadAppConfig(app("ffi-none", {})).permissions).toBeUndefined();
  for (const path of [
    "lib/db",
    "$APP/../db",
    "$APP//db",
    "C:/db.dll",
    "$SYSTEM/sub/db",
    "$PATH/db",
    { ios: "$APP/db" },
  ]) {
    expect(() => loadAppConfig(app("ffi-bad", { permissions: { ffi: [{ name: "db", path }] } }))).toThrow(ConfigError);
  }
  expect(() => loadAppConfig(app("ffi-key", { permissions: { files: [] } }))).toThrow(ConfigError);
});

test('"permissions.process" keeps the path of the platform and its argument rules, and refuses batch files and bad patterns', () => {
  const process = [
    {
      name: "git",
      path: { windows: "C:/Program Files/Git/bin/git.exe", linux: "/usr/bin/git" },
      args: ["status", { pattern: "--porcelain" }],
    },
    { name: "tool", path: "$APP/bin/tool", args: true },
    { name: "where", path: "$SYSTEM/where", args: false },
  ];
  const on = (platform: "windows" | "macos") =>
    loadAppConfig(app(`process-${platform}`, { permissions: { process } }), { platform }).permissions?.process;
  expect(on("windows")).toEqual([
    { name: "git", path: "C:/Program Files/Git/bin/git.exe", args: ["status", { pattern: "--porcelain" }] },
    { name: "tool", path: "$APP/bin/tool", args: true },
    { name: "where", path: "$SYSTEM/where" },
  ]);
  expect(on("macos")?.map((e) => e.name)).toEqual(["tool", "where"]);
  const bad = (entry: Record<string, unknown>, platform: "windows" | "linux" = "windows") =>
    expect(() => loadAppConfig(app("process-bad", { permissions: { process: [entry] } }), { platform })).toThrow(
      ConfigError,
    );
  for (const path of [
    "bin/tool",
    "$APP/../tool",
    "$PATH/sub/tool",
    "$SYSTEM/../cmd",
    "$APP/run.bat",
    "C:/x/run.cmd",
    "//host/x.exe",
  ]) {
    bad({ name: "t", path });
  }
  bad({ name: "t", path: { linux: "C:/x" } }, "linux");
  bad({ name: "t", path: "$APP/t", args: "any" });
  bad({ name: "t", path: "$APP/t", args: [{ pattern: "(" }] });
  bad({ name: "t", path: "$APP/t", args: [{ pattern: "(?=x)" }] });
  bad({ name: "t", path: "$APP/t", args: [{ regex: "x" }] });
  bad({ name: ".t", path: "$APP/t" });
});

test('"intl" is "full" unless the config says "invariant", and nothing else is accepted', () => {
  expect(loadAppConfig(app("intl-default", {})).intl).toBe("full");
  expect(loadAppConfig(app("intl-invariant", { intl: "invariant" })).intl).toBe("invariant");
  for (const intl of ["none", true, "Invariant"]) {
    expect(() => loadAppConfig(app("intl-bad", { intl }))).toThrow(ConfigError);
  }
});

test("the ICU pack holds the data file alone, page-aligned for the host to map", () => {
  const dat = join(root, "icudt79l.dat");
  writeFileSync(dat, "pretend ICU data");
  const pack = readPayload(writeIcuPack(dat));
  expect(pack.entries.map((e) => [e.kind, e.name])).toEqual([[KIND.ICU_DATA, "sk/icu/icudt79l.dat"]]);
  expect(pack.entries[0]!.start % PAGE).toBe(0);
  expect(new TextDecoder().decode(pack.bytes(pack.entries[0]!))).toBe("pretend ICU data");
});

test("splitPacks deals the assets out in the packs' order", () => {
  const f = (key: string) => ({ key, path: key, size: 1 });
  const files = ["assets/fonts/a.woff2", "assets/fonts/b.ttf", "assets/logo.png", "sounds/x.wav"].map(f);
  const split = splitPacks(files, [
    { name: "fonts", match: ["**/*.woff2", "**/*.ttf"] },
    { name: "media", match: ["sounds/**"] },
  ]);
  expect(split.packs.map((p) => [p.name, p.files.map((x) => x.key)])).toEqual([
    ["fonts", ["assets/fonts/a.woff2", "assets/fonts/b.ttf"]],
    ["media", ["sounds/x.wav"]],
  ]);
  expect(split.own.map((x) => x.key)).toEqual(["assets/logo.png"]);
  const rest = splitPacks(files, [
    { name: "fonts", match: ["**/*.woff2"] },
    { name: "assets", match: null },
  ]);
  expect(rest.own).toEqual([]);
  expect(rest.packs[1]!.files.map((x) => x.key)).toEqual(["assets/fonts/b.ttf", "assets/logo.png", "sounds/x.wav"]);
});

test("sk build writes the packs beside the payload and names them, with their ids, in its META", () => {
  const at = app("built", {
    assets: ["assets"],
    packs: [
      { name: "fonts", match: ["**/*.woff2", "**/*.ttf"] },
      { name: "assets", match: ["**/*.wav"] },
      { name: "empty", match: ["nothing/**"] },
    ],
  });
  const out = join(at, "dist");
  const cli = resolve(import.meta.dir, "../cli/sk.ts");
  const result = Bun.spawnSync([process.execPath, cli, "build", at, "--out", out, "--payload-only", "--json"], {
    stdout: "pipe",
    stderr: "pipe",
  });
  expect(result.exitCode, new TextDecoder().decode(result.stderr)).toBe(0);
  const output = new TextDecoder().decode(result.stdout);
  const summary = JSON.parse(output.trim().split("\n").pop()!);
  expect(summary.packs.map((p: { name: string; assets: number }) => [p.name, p.assets])).toEqual([
    ["fonts", 2],
    ["assets", 1],
  ]);

  const payload = readPayload(new Uint8Array(readFileSync(summary.skpk)));
  const assets = payload.entries.filter((e) => e.kind === KIND.ASSET).map((e) => e.name);
  expect(assets).toEqual(["assets/logo.png"]);
  const meta = JSON.parse(new TextDecoder().decode(payload.bytes(payload.find(KIND.META, NAMES.META)!)));
  expect(meta.assets).toBe(4);
  expect(meta.packs.map((p: { name: string; file: string }) => [p.name, p.file])).toEqual([
    ["fonts", "fonts.skpk"],
    ["assets", "assets.skpk"],
  ]);

  for (const ref of meta.packs as { name: string; file: string; id: string }[]) {
    const blob = new Uint8Array(readFileSync(join(out, ".sk", ref.file)));
    const pack = readPayload(blob);
    expect(pack.entries.every((e) => e.kind === KIND.ASSET)).toBe(true);
    // The data is page-aligned in the file, which the host maps from 0.
    expect(pack.dataOffset % PAGE).toBe(0);
    const index = pack.index;
    expect(ref.id).toBe(createHash("sha256").update(index).digest("hex"));
  }
  const fonts = readPayload(new Uint8Array(readFileSync(join(out, ".sk", "fonts.skpk"))));
  expect(fonts.entries.map((e) => e.name)).toEqual(["assets/fonts/a.woff2", "assets/fonts/b.ttf"]);
  expect(new TextDecoder().decode(fonts.bytes(fonts.entries[0]!))).toBe("font a");
}, 120_000);
