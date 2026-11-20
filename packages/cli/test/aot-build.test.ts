import { afterAll, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  compileNative,
  compilerInfo,
  compilerMismatch,
  nativeManifest,
  repoSkAot,
  removedFeatureProblem,
  type NativeImage,
} from "../aot/native";
import { loadPin, pinnedCompilerInfo } from "../aot/pin";
import { closedUses } from "../aot/closed";
import { nativeInput, opaqueGlobals, opaqueHost, TABLE_ID, TRANSFORM } from "../aot/transform";
import { removedFeatures, type FeatureUse, type Uses } from "../aot/uses";
import { aotCheck, aotFlags, aotPlatformProblem, hostAot, resolveAotCompiler } from "../cli/aot-build";
import { buildApp, PROD_TS } from "../cli/build-app";
import { ConfigError, loadAppConfig } from "../cli/config";
import { HostError, type HostInfo, type ResolvedHost } from "../cli/host";
import { quietProgress } from "../cli/term";
import { RULES } from "./check-fixtures";

describe("strict image reachability", () => {
  const none = (): Uses => ({ features: [], unseen: [], entryModules: [1], statistics: {} });
  const inspect = (source: string, witness = "Array.isArray", closed = none()) => {
    const input = nativeInput(source, ["", ""]);
    const section = input.sections.at(-1)!;
    const at = section.byteOffset + Buffer.byteLength(source.slice(0, source.indexOf(witness)));
    const open = none();
    open.unseen.push({ cause: "EvaluatesText", builtin: "Function", module: 0, position: at });
    return closedUses(closed, open, input);
  };
  test("a locked predicate reference can be distinguished from calling a constructor after it", () => {
    const base = "/* 🪟 */ let isArray=Array.isArray; isArray([]);";
    expect(inspect(base, "isArray=").unseen).toEqual([]);
    expect(inspect(base + "new Function(window.code)", "isArray=").unseen.length).toBeGreaterThan(0);
    expect(inspect(base + "(0,eval)(window.code)", "isArray=").unseen.length).toBeGreaterThan(0);
    expect(inspect(base + "(async()=>{}).constructor(window.code)", "isArray=").unseen.length).toBeGreaterThan(0);
    expect(inspect(base + "import(window.path)", "isArray=").unseen.length).toBeGreaterThan(0);
    expect(inspect(base + "globalThis[window.key](window.code)", "isArray=").unseen.length).toBeGreaterThan(0);
    expect(
      inspect(base + "let run; run=Reflect.get(window, window.key); Reflect.apply(run,null,[])", "isArray=").unseen
        .length,
    ).toBeGreaterThan(0);
  });
  test("preserves unknown causes, shadowed globals and unresolved witness locations", () => {
    const source = "function f(Array) { let pred=Array.isArray; return pred(1) }";
    expect(inspect(source, "pred=").unseen).toHaveLength(1);
    expect(inspect("let pred=Array.isArray; pred=other", "pred=").unseen).toHaveLength(1);
    const closed = none();
    closed.unseen.push({ cause: "RunsUnseenCode", builtin: null, module: 0xffffffff, position: 0 });
    expect(inspect("let pred=Array.isArray", "pred=", closed).unseen).toEqual(closed.unseen);
    expect(inspect("let pred=Array.isArray", "Array.isArray").unseen).toHaveLength(1);
  });
  test("ignores unrelated object methods and locally scoped function names", () => {
    const source = "function f(Function) { return Function(3) } const obj={eval(x) { return x }}; obj.eval(2);";
    expect(closedUses(none(), none(), nativeInput(source, ["", ""])).unseen).toEqual([]);
    expect(closedUses(none(), none(), nativeInput('const k="setTimeout"; globalThis[k](()=>{},0)')).unseen).toEqual([]);
    expect(closedUses(none(), none(), nativeInput('Reflect.get(window,"Function")(window.text)')).unseen).not.toEqual(
      [],
    );
  });
  test("removes only unnamed Bun services from the closed Interpreter report", () => {
    const closed = none();
    const base: FeatureUse = {
      feature: "Interpreter",
      id: 1,
      kind: "Unnamed",
      builtin: "Bun.Transpiler",
      module: 0xffffffff,
      position: 0,
    };
    const named = { ...base, kind: "Named" };
    const unknown = { ...base, builtin: "AnotherBuiltin" };
    closed.features.push(base, named, unknown);
    expect(closedUses(closed, none(), nativeInput("0", ["", ""])).features).toEqual([named, unknown]);
  });
});

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

  test("the exact pair of host preludes is bound to its digest", () => {
    const preludeScripts = ["globalThis.first = 1;", "globalThis.second = 2;"] as const;
    const preludeSha256 = createHash("sha256").update(preludeScripts.join("\0")).digest("hex");
    expect(hostAot(host({ aot: { ...aot, preludeScripts, preludeSha256 } })).preludeScripts).toEqual(preludeScripts);
    expect(() => hostAot(host({ aot: { ...aot, preludeScripts } }))).toThrow("preludeSha256");
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
      uses: null,
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
  test("a bundled app reads the observers and style API installed by its runtime", () => {
    const root = app(
      {},
      `
      Object.assign(globalThis, {
        ResizeObserver: class { value = "resize" },
        MutationObserver: class { value = "mutation" },
        IntersectionObserver: class { value = "intersection" },
        getComputedStyle: () => "style"
      });
      function local(ResizeObserver) { return new ResizeObserver().value; }
      globalThis.readObservers = () => [
        new ResizeObserver().value, new MutationObserver().value, new IntersectionObserver().value,
        getComputedStyle(), local(class { value = "local" })
      ];
    `,
    );
    const output = join(root, "app.prod.js");
    const result = Bun.spawnSync([process.execPath, PROD_TS, join(root, "src", "main.ts"), output]);
    expect(result.exitCode).toBe(0);
    const browser = { Object, readObservers: undefined as (() => string[]) | undefined };
    new Function("__SK_AOT_HOST__", opaqueHost(readFileSync(output, "utf8")))(browser);
    expect(browser.readObservers?.()).toEqual(["resize", "mutation", "intersection", "style", "local"]);
  });

  test("browser bundles resolve CommonJS from their embedding realm", async () => {
    const root = app(
      {},
      `
      import answer from "./answer.cjs";
      globalThis.answer = answer;
      globalThis.lookup = () => require("host-extension");
    `,
    );
    writeFileSync(join(root, "src", "answer.cjs"), "module.exports = 42;");
    const build = await Bun.build({
      entrypoints: [join(root, "src", "main.ts")],
      target: "browser",
      format: "iife",
      minify: true,
      external: ["host-extension"],
    });
    expect(build.success).toBe(true);
    const prelude = "globalThis.commonjs = [typeof require, typeof module, typeof exports];";
    const { code } = nativeInput(await build.outputs[0]!.text(), [prelude, ""]);
    const evaluate = new Function("__SK_AOT_HOST__", "require", "module", "exports", code);
    const browser: { Object: ObjectConstructor; commonjs?: string[]; answer?: number; lookup?: () => string } = {
      Object,
    };
    const ambientRequire = () => "outer module loader";
    evaluate(browser, ambientRequire, {}, {});
    expect(browser.commonjs).toEqual(["undefined", "undefined", "undefined"]);
    expect(browser.answer).toBe(42);
    expect(() => browser.lookup?.()).toThrow("Dynamic require");

    const provided = { ...browser, require: (name: string) => name, module: {}, exports: {} };
    evaluate(provided, ambientRequire, {}, {});
    expect(provided.commonjs).toEqual(["function", "object", "object"]);
    expect(provided.lookup?.()).toBe("host-extension");
  });

  test("each script sees the globals installed by the preceding one", () => {
    const preludes = ["globalThis.matchMedia = () => 21;", "globalThis.answer = matchMedia() * 2;"] as const;
    const app = "globalThis.result = globalThis.answer;";
    const { code, sections } = nativeInput(app, preludes);
    const runtime: Record<string, unknown> = {};
    new Function("__SK_AOT_HOST__", code)(runtime);
    expect(runtime.result).toBe(42);
    const input = Buffer.from(code);
    expect(
      sections.map((s) => input.toString("utf8", s.byteOffset, s.byteOffset + Buffer.byteLength(s.source))),
    ).toEqual([...preludes, app]);
    expect(nativeInput(app, undefined, true).code).toBe(opaqueHost(app));
  });
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

describe("images for reduced workloads", () => {
  const source = "const o = {}; for (const k in o) o[k](); const { Module } = globalThis.WebAssembly;\n";
  const at = (text: string) => nativeInput(source).code.indexOf(text);
  const use = (kind: string, builtin: string, position: number): FeatureUse => ({
    feature: "WebAssembly",
    id: 4,
    kind,
    builtin,
    module: 0,
    position,
  });
  const image = (features: FeatureUse[]): NativeImage => ({
    image: new Uint8Array(1),
    entry: 1,
    identity: "00".repeat(16),
    imageSha256: "",
    sourceBundleSha256: "",
    nativeInputSha256: "",
    irSha256: "",
    diagnostics: "",
    uses: { entryModules: [1], features, unseen: [], statistics: {} } satisfies Uses,
  });
  const dynamicCall = [use("Unnamed", "WebAssembly", at("o[k]")), use("FoundByName", "WebAssembly.Module", at("o[k]"))];

  test("what a dynamic call might reach does not refuse the image", () => {
    expect(removedFeatureProblem(image(dynamicCall), source, ["WebAssembly"], "aot-nojit")).toBeNull();
  });

  test("a read of WebAssembly refuses it and names the place", () => {
    const problem = removedFeatureProblem(
      image([...dynamicCall, use("FoundByName", "WebAssembly", at("WebAssembly;"))]),
      source,
      ["WebAssembly"],
      "aot-nojit",
    );
    expect(problem).toContain("app.prod.js 1:" + (source.indexOf("WebAssembly;") + 1));
  });

  test("an sk-aot that records nothing refuses it", () => {
    expect(removedFeatureProblem({ ...image([]), uses: null }, source, ["WebAssembly"], "aot-nojit")).not.toBeNull();
  });

  test("a use in the host's window prelude points at its own source", () => {
    const preludes = ["/* 中文 */", "'🦀Я'; WebAssembly.Module"] as const;
    const atWindow = nativeInput(source, preludes).sections[1]!.byteOffset + Buffer.byteLength("'🦀Я'; ");
    expect(
      removedFeatureProblem(
        image([use("Named", "WebAssembly", atWindow)]),
        source,
        ["WebAssembly"],
        "aot-nojit",
        preludes,
      ),
    ).toContain("at sk://window.js 1:8");
  });

  test("a host without the interpreter refuses unseen code even without a named feature use", () => {
    const removed = removedFeatures(["native-aot", "no-interpreter"]);
    expect(removed).toEqual(["Interpreter"]);
    for (const cause of ["EvaluatesText", "LoadsModuleByName"]) {
      const native = image([]);
      native.uses!.unseen.push({ cause, builtin: null, module: 0, position: at("o[k]") });
      expect(removedFeatureProblem(native, source, removed, "strict-aot")).toContain(
        cause + " (code outside the image) at app.prod.js 1:",
      );
      expect(removedFeatureProblem(native, source, ["WebAssembly"], "aot-nojit")).toBeNull();
    }
  });

  test("a possible interpreter entry refuses only a workload without it", () => {
    const native = image([{ ...use("Unnamed", "Function", at("o[k]")), feature: "Interpreter", id: 1 }]);
    expect(removedFeatureProblem(native, source, ["Interpreter"], "strict-aot")).toContain("Function at app.prod.js");
    expect(removedFeatureProblem(image([]), source, ["Interpreter"], "strict-aot")).toBeNull();
  });
});
