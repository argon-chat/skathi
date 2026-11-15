import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  compileNative,
  compilerInfo,
  compilerMismatch,
  nativeManifest,
  repoSkAot,
  type NativeImage,
} from "../aot/native";
import { loadPin, pinnedCompilerInfo } from "../aot/pin";
import { opaqueGlobals, TABLE_ID, TRANSFORM } from "../aot/transform";
import { aotCheck, aotFlags, aotPlatformProblem, hostAot, resolveAotCompiler } from "../cli/aot-build";
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
    source: "--workload",
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
    expect(() => loadAppConfig(app({ aot: "yes" }))).toThrow(ConfigError);
  });

  test("--aot and --no-aot override the config; both is an error", () => {
    expect(aotFlags({}).aot).toBeUndefined();
    expect(aotFlags({ aot: true }).aot).toBe(true);
    expect(aotFlags({ "no-aot": true }).aot).toBe(false);
    expect(() => aotFlags({ aot: true, "no-aot": true })).toThrow();
    expect(aotFlags({ "aot-compiler": "tools/sk-aot.exe" }).aotCompiler).toBe(resolve("tools/sk-aot.exe"));
    expect(() => aotFlags({ "aot-compiler": " " })).toThrow();
    expect(() => aotFlags({ "aot-compiler": true })).toThrow();
  });

  test("only a Windows x64 build on Windows x64 can be AOT-compiled", () => {
    expect(aotPlatformProblem("windows", { platform: "win32", arch: "x64" })).toBeNull();
    for (const [target, here] of [
      ["macos", { platform: "darwin", arch: "arm64" }],
      ["linux", { platform: "win32", arch: "x64" }],
      ["windows", { platform: "win32", arch: "arm64" }],
      ["windows", { platform: "linux", arch: "x64" }],
    ] as const)
      expect(aotPlatformProblem(target, here)).not.toBeNull();
  });

  test("a non-Windows AOT build fails before a host is looked for", async () => {
    const root = app({ aot: true });
    const error = await buildApp({ target: root, platform: "linux", log: () => {} }).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ConfigError);
    const payloadOnly = await buildApp({ target: root, payloadOnly: true, log: () => {} }).catch((e: unknown) => e);
    expect(payloadOnly).toBeInstanceOf(ConfigError);
  });
});

describe("the host decides what an image is compiled against", () => {
  test("a host without the AOT loader is refused, saying what to use instead", () => {
    expect(hostAot(host({}))).toEqual(aot);
    expect(() => hostAot(host({ features: ["prod"], aot: null }))).toThrow(HostError);
    expect(() => hostAot(host({ features: ["prod"] }))).toThrow(HostError);
    expect(() => hostAot(host({ aot: { ...aot, transform: "opaque-host-v0" } }))).toThrow(HostError);
  });

  test("a compiler must agree with the host on every field", () => {
    expect(compilerMismatch(pinnedCompilerInfo(pin), aot)).toBeNull();
    for (const name of ["target", "frontend", "engine", "tableId", "compileOptions"] as const)
      expect(compilerMismatch({ ...pinnedCompilerInfo(pin), [name]: "stale" }, aot)).not.toBeNull();
  });

  test("a named sk-aot wins and must exist; SK_AOT_COMPILER names one too", async () => {
    const missing = join(tmpdir(), `sk-aot-missing-${process.pid}.exe`);
    await expect(resolveAotCompiler(aot, missing, () => {})).rejects.toThrow(missing);
    const before = process.env.SK_AOT_COMPILER;
    process.env.SK_AOT_COMPILER = missing;
    try {
      await expect(resolveAotCompiler(aot, undefined, () => {})).rejects.toThrow(missing);
    } finally {
      if (before === undefined) delete process.env.SK_AOT_COMPILER;
      else process.env.SK_AOT_COMPILER = before;
    }
  });

  const built = repoSkAot(resolve(import.meta.dir, "../../.."));
  test.skipIf(!existsSync(built))("the checkout's sk-aot build reports the pinned toolchain", async () => {
    expect(await compilerInfo(built)).toEqual(pinnedCompilerInfo(pin));
  });

  test.skipIf(!existsSync(built))(
    "the checkout's sk-aot compiles a bundle inside a package",
    async () => {
      const root = app({});
      writeFileSync(join(root, "package.json"), '{"name":"aot-test","type":"module"}\n');
      const native = await compileNative({
        compiler: built,
        source: "globalThis.answer = 6 * 7;\n",
        work: join(root, "dist", ".sk", "aot"),
      });
      expect(native.entry).toBe(1);
      expect(native.image.length).toBeGreaterThan(0);
    },
    60_000,
  );

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
});

describe("sk check gates an AOT build", () => {
  test("errors refuse the build and print the report", async () => {
    const error = await aotCheck(RULES, undefined, quietProgress, false).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ConfigError);
    const message = (error as Error).message;
    expect(message).toMatch(/SK\d{4}/);
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
