import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { compilerMismatch, nativeManifest, tarEntry, type NativeImage } from "../aot/native";
import { loadPin, pinnedCompilerInfo } from "../aot/pin";
import { opaqueGlobals, opaqueHost, TABLE_ID, TRANSFORM } from "../aot/transform";
import { aotCheck, aotFlags, aotPlatformProblem, hostAot } from "../cli/aot-build";
import { buildApp } from "../cli/build-app";
import { ConfigError, loadAppConfig } from "../cli/config";
import { HostError, type HostInfo, type ResolvedHost } from "../cli/host";
import { quietProgress } from "../cli/term";
import { RULES } from "./check-fixtures";

const dirs: string[] = [];
afterAll(() => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));
function app(config: Record<string, unknown>, main = "console.log('hi');\n"): string {
  const root = mkdtempSync(join(tmpdir(), "sk-aot-"));
  dirs.push(root);
  mkdirSync(join(root, "src"));
  writeFileSync(join(root, "src", "main.ts"), main);
  writeFileSync(join(root, "sk.config.json"), JSON.stringify({ name: "aot-test", version: "1.0.0", ...config }));
  return root;
}

const pin = loadPin();
const aot = { ...pinnedCompilerInfo(pin), transform: TRANSFORM, preludeSha256: "p".repeat(64) };
function host(info: Partial<HostInfo>): ResolvedHost {
  return {
    exe: "C:/hosts/sk-host.exe",
    source: "--host",
    info: {
      mode: "prod",
      skathi: "0.0.0",
      crate: "0.1.0",
      variant: "standard",
      devtools: true,
      features: ["prod", "native-aot"],
      jscPin: pin.engine.revision,
      payloadFormat: 1,
      profile: "release",
      aot,
      ...info,
    },
  };
}

describe("flags and config", () => {
  test('"aot" in sk.config.json is a boolean, off by default', () => {
    expect(loadAppConfig(app({ aot: true })).aot).toBe(true);
    expect(loadAppConfig(app({})).aot).toBe(false);
    expect(() => loadAppConfig(app({ aot: "yes" }))).toThrow('"aot" must be true or false');
  });

  test("--aot and --no-aot override the config; both is an error", () => {
    expect(aotFlags({}).aot).toBeUndefined();
    expect(aotFlags({ aot: true }).aot).toBe(true);
    expect(aotFlags({ "no-aot": true }).aot).toBe(false);
    expect(() => aotFlags({ aot: true, "no-aot": true })).toThrow("contradict");
    expect(aotFlags({ "aot-compiler": "tools/aot-image.exe" }).aotCompiler).toBe(resolve("tools/aot-image.exe"));
    expect(() => aotFlags({ "aot-compiler": " " })).toThrow("--aot-compiler needs");
    expect(() => aotFlags({ "aot-compiler": true })).toThrow("--aot-compiler needs");
  });

  test("only a Windows x64 build on Windows x64 can be AOT-compiled", () => {
    expect(aotPlatformProblem("windows", { platform: "win32", arch: "x64" })).toBeNull();
    for (const [target, here] of [
      ["macos", { platform: "darwin", arch: "arm64" }],
      ["linux", { platform: "win32", arch: "x64" }],
      ["windows", { platform: "win32", arch: "arm64" }],
      ["windows", { platform: "linux", arch: "x64" }],
    ] as const)
      expect(aotPlatformProblem(target, here)).toContain("AOT builds are Windows x64 only for now");
  });

  test("a non-Windows AOT build fails before a host is looked for", async () => {
    const root = app({ aot: true });
    const error = await buildApp({ target: root, platform: "linux", log: () => {} }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ConfigError);
    expect((error as Error).message).toContain("AOT builds are Windows x64 only for now");
    const payloadOnly = await buildApp({ target: root, payloadOnly: true, log: () => {} }).catch((e: unknown) => e);
    expect((payloadOnly as Error).message).toMatch(/AOT builds are Windows x64 only|--payload-only/);
  });
});

describe("the host decides what an image is compiled against", () => {
  test("a host without the AOT loader is refused, saying what to use instead", () => {
    expect(hostAot(host({}))).toEqual(aot);
    expect(() => hostAot(host({ features: ["prod"], aot: null }))).toThrow(HostError);
    expect(() => hostAot(host({ features: ["prod"] }))).toThrow("built without the native AOT loader");
    expect(() => hostAot(host({ aot: { ...aot, transform: "opaque-host-v0" } }))).toThrow("transform");
  });

  test("a compiler must agree with the host on every field", () => {
    expect(compilerMismatch(pinnedCompilerInfo(pin), aot)).toBeNull();
    for (const name of ["target", "frontend", "engine", "tableId", "compileOptions"] as const)
      expect(compilerMismatch({ ...pinnedCompilerInfo(pin), [name]: "stale" }, aot)).toStartWith(`${name} stale`);
  });

  test("the application manifest names everything the host checks", () => {
    const native: NativeImage = {
      image: new Uint8Array(10),
      entry: 1,
      identity: "00".repeat(16),
      imageSha256: "i".repeat(64),
      sourceBundleSha256: "s".repeat(64),
      nativeInputSha256: "n".repeat(64),
      irSha256: "r".repeat(64),
      diagnostics: "",
    };
    const manifest = nativeManifest("application", pinnedCompilerInfo(pin), native, {
      image: "sk/app.image",
      preludeSha256: aot.preludeSha256,
    });
    expect(manifest).toMatchObject({
      schema: "skathi.native-aot",
      version: 1,
      stage: "native-image",
      kind: "application",
      ...pinnedCompilerInfo(pin),
      transform: TRANSFORM,
      imageBytes: 10,
      entry: 1,
      preludeSha256: aot.preludeSha256,
      sourceBundleSha256: native.sourceBundleSha256,
    });
  });
});

describe("the opaque-host transform", () => {
  test("its host globals come from the pinned table", () => {
    expect(TABLE_ID).toBe(pin.table.id);
    const names = opaqueGlobals();
    expect(names).toContain("fetch");
    expect(names).toContain("document");
    expect([...names].sort()).toEqual(names);
  });

  test("wraps the bundle so globalThis and the host globals are the host's", () => {
    const wrapped = opaqueHost("app();", ["a", "b"]);
    expect(wrapped).toBe(
      `// ${TRANSFORM}: stable host API references are opaque inputs.\n(function(__sk_host) {\n` +
        "const globalThis = __sk_host;\nconst {a,b} = __sk_host;\napp();\n})(__SK_AOT_HOST__);\n",
    );
  });
});

test("one file comes out of a tar, ustar prefix and pax path included", () => {
  const block = (fields: { name: string; size: number; type: string; prefix?: string }) => {
    const header = new Uint8Array(512);
    const put = (at: number, text: string) => header.set(new TextEncoder().encode(text), at);
    put(0, fields.name);
    put(124, fields.size.toString(8).padStart(11, "0"));
    put(156, fields.type);
    put(257, "ustar");
    if (fields.prefix) put(345, fields.prefix);
    return header;
  };
  const padded = (bytes: Uint8Array) => {
    const out = new Uint8Array(Math.ceil(bytes.length / 512) * 512);
    out.set(bytes);
    return out;
  };
  const enc = (text: string) => new TextEncoder().encode(text);
  const pax = enc("28 path=package/bin/bun.exe\n");
  const tar = new Uint8Array([
    ...block({ name: "package/README.md", size: 3, type: "0" }),
    ...padded(enc("doc")),
    ...block({ name: "PaxHeader", size: pax.length, type: "x" }),
    ...padded(pax),
    ...block({ name: "truncated", size: 4, type: "0" }),
    ...padded(enc("MZ!!")),
    ...block({ name: "x.txt", size: 1, type: "0", prefix: "package/lib" }),
    ...padded(enc("x")),
    ...new Uint8Array(1024),
  ]);
  expect(new TextDecoder().decode(tarEntry(tar, "package/bin/bun.exe")!)).toBe("MZ!!");
  expect(new TextDecoder().decode(tarEntry(tar, "package/lib/x.txt")!)).toBe("x");
  expect(tarEntry(tar, "truncated")).toBeNull();
  expect(tarEntry(tar, "missing")).toBeNull();
});

describe("sk check gates an AOT build", () => {
  test("errors refuse the build and print the report", async () => {
    const error = await aotCheck(RULES, undefined, quietProgress, false).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ConfigError);
    const message = (error as Error).message;
    expect(message).toContain("sk check found what a native image cannot run");
    expect(message).toMatch(/SK\d{4}/);
    expect(message).toContain("--no-aot");
  }, 60_000);

  test("a clean app passes, its warnings printed", async () => {
    const warned: string[] = [];
    const progress = { ...quietProgress, warn: (text: string) => warned.push(text) };
    const report = await aotCheck(app({}), undefined, progress, false);
    expect(report.summary.errors).toBe(0);
    expect(warned).toHaveLength(report.summary.warnings);
  }, 60_000);
});

test("the CLI package carries the pin and the transform's table", () => {
  const pkg = JSON.parse(readFileSync(resolve(import.meta.dir, "../package.json"), "utf8")) as { files: string[] };
  expect(pkg.files).toContain("aot");
});
