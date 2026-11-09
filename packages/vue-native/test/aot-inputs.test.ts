import { afterAll, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import ts from "typescript";
import { AotInputs, aotEngineIdentity } from "../aot-inputs";
import { NAMES, readVnpk } from "../cli/vnpk";

const scratch = mkdtempSync(join(tmpdir(), "vn-aot-inputs-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));
function file(path: string, text: string) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
}
const hash = (text: string | Uint8Array) => createHash("sha256").update(text).digest("hex");

test("engine identity distinguishes plain and AOT assets at the same revision", () => {
  const shared = "webkit.version = same-revision\n";
  const plain = aotEngineIdentity(shared + "webkit.asset = bun-webkit-windows-amd64.tar.gz\nwebkit.aot = 0 # plain\nwebkit.sha256 = " + "1".repeat(64));
  const aot = aotEngineIdentity(shared + "webkit.asset = bun-webkit-windows-amd64-aot.tar.gz\nwebkit.aot = 1\nwebkit.sha256 = " + "2".repeat(64));
  expect(plain.jscPin).toBe(aot.jscPin);
  expect(plain.aot).toBe(false);
  expect(aot.aot).toBe(true);
  expect(plain.asset).not.toBe(aot.asset);
  expect(plain.sha256).not.toBe(aot.sha256);
  expect(aotEngineIdentity(null)).toEqual({ jscPin: null, asset: null, aot: null, sha256: null });
  expect(() => aotEngineIdentity("webkit.aot=maybe")).toThrow(/Invalid webkit.aot/);
});

test("app paths cannot impersonate a named dependency root", () => {
  const app = join(scratch, "anchor-collision-app");
  const sdk = join(scratch, "anchor-collision-sdk");
  const out = join(scratch, "anchor-collision-out");
  const entry = join(app, "main.ts");
  file(entry, "export {};");
  file(join(app, "@sdk/lib.ts"), "export const appValue = 1;");
  file(join(sdk, "lib.ts"), "export const sdkValue = 2;");
  const archive = new AotInputs({ out, appRoot: app, entry, typescript: ts, aliases: {}, sourceRoots: { sdk, toolchain: resolve(import.meta.dir, "..") } });
  archive.source(entry, readFileSync(entry, "utf8"));
  archive.source(join(app, "@sdk/lib.ts"), readFileSync(join(app, "@sdk/lib.ts"), "utf8"));
  archive.source(join(sdk, "lib.ts"), readFileSync(join(sdk, "lib.ts"), "utf8"));
  expect(() => archive.write({ script: "", scriptUrl: "vn://app.prod.js", defines: {}, engine: aotEngineIdentity(null), sdkVersion: "test" })).toThrow(/source-root anchor/);
  expect(existsSync(out)).toBe(false);
}, 30_000);

test.each(["ts", "original.ts", "js", "original.js"])("virtual Vue modules reject real .vue.%s files instead of hiding type imports", (suffix) => {
  const app = join(scratch, `vue-collision-${suffix}`);
  const path = join(app, `Pane.vue.${suffix}`);
  const original = 'import type { Hidden } from "./hidden-types"; export {};';
  file(path, original);
  const archive = new AotInputs({ out: join(app, "out"), appRoot: app, entry: path, typescript: ts, aliases: {} });
  archive.source(path, original);
  expect(() => archive.vue(join(app, "Pane.vue"), "<template/>", "export default {}", suffix.endsWith("ts") ? "ts" : "js", "const value = 1;")).toThrow(/collides with a real source file/);
  expect(readFileSync(path, "utf8")).toBe(original);
  expect(existsSync(join(app, "out"))).toBe(false);
});

test("canonical and decomposed Unicode filenames retain deterministic inventory order", () => {
  const app = join(scratch, "unicode-order-app");
  const entry = join(app, "main.ts");
  file(entry, "export {};");
  const names = ["é.ts", "e\u0301.ts"];
  for (const name of names) file(join(app, name), "export const value = 1;");
  const build = (out: string, order: string[]) => {
    const archive = new AotInputs({ out, appRoot: app, entry, typescript: ts, aliases: {}, sourceRoots: { sdk: resolve(import.meta.dir, "..") } });
    archive.source(entry, readFileSync(entry, "utf8"));
    for (const name of order) archive.source(join(app, name), readFileSync(join(app, name), "utf8"));
    return readFileSync(archive.write({ script: "", scriptUrl: "vn://app.prod.js", defines: {}, engine: aotEngineIdentity(null), sdkVersion: "test" }), "utf8");
  };
  const first = build(join(scratch, "unicode-first"), names);
  const second = build(join(scratch, "unicode-second"), [...names].reverse());
  expect(first).toBe(second);
  expect(JSON.parse(first).files.filter((input: any) => names.includes(input.path))).toHaveLength(2);
}, 30_000);

test("runtime aliases retain package declaration entries and their type-only closure", () => {
  const app = join(scratch, "package-declarations-app");
  const out = join(scratch, "package-declarations-out");
  const entry = join(app, "src/main.ts");
  const runtime = join(app, "node_modules/native-package/index.js");
  file(entry, 'import type { Value } from "native-package"; export const result: Value = 1;');
  file(join(app, "tsconfig.json"), '{"files":["src/main.ts"],"compilerOptions":{"moduleResolution":"bundler","module":"esnext","types":[]}}');
  file(join(app, "node_modules/native-package/package.json"), '{"name":"native-package","main":"index.js","types":"types/entry.d.ts"}');
  file(runtime, "exports.nativeRuntime = true;");
  file(join(app, "node_modules/native-package/types/entry.d.ts"), 'export type { Value } from "./hidden";');
  file(join(app, "node_modules/native-package/types/hidden.d.ts"), "export type Value = number;");
  const archive = new AotInputs({ out, appRoot: app, entry, typescript: ts, aliases: { "native-package": runtime }, sourceRoots: { sdk: resolve(import.meta.dir, "..") } });
  archive.source(entry, readFileSync(entry, "utf8"));
  archive.source(runtime, readFileSync(runtime, "utf8"));
  const path = archive.write({ script: "const result = 1;", scriptUrl: "vn://app.prod.js", defines: {}, engine: aotEngineIdentity(null), sdkVersion: "test" });
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  const paths = manifest.files.map((input: any) => input.path);
  expect(paths).toContain("node_modules/native-package/index.js");
  expect(paths).toContain("node_modules/native-package/types/entry.d.ts");
  expect(paths).toContain("node_modules/native-package/types/hidden.d.ts");
});

test("archive preserves types erased by bundling, extended config, declarations and unresolved imports", () => {
  const app = join(scratch, "typed-project");
  const out = join(scratch, "archive");
  const entry = join(app, "src/main.ts");
  file(entry, 'import type { Value } from "../types/only-type"; import "./Pane.vue"; export const value: Value = 42;');
  file(join(app, "types/only-type.d.ts"), "export type Value = number; ");
  file(join(app, "types/props.d.ts"), "export interface Props { count: number } ");
  file(join(app, "src/ambient.d.ts"), "declare const ambientToken: unique symbol;");
  file(join(app, "tsconfig.base.json"), '{"compilerOptions":{"moduleResolution":"bundler","module":"esnext","types":[]}}');
  file(join(app, "tsconfig.json"), '{"extends":"./tsconfig.base.json","files":["src/main.ts","src/ambient.d.ts"]}');
  const archive = new AotInputs({ out, appRoot: app, entry, typescript: ts, aliases: {}, sourceRoots: { sdk: resolve(import.meta.dir, "..") } });
  archive.source(entry, readFileSync(entry, "utf8"));
  archive.vue(join(app, "src/Pane.vue"), '<script setup lang="ts">import type { Props } from "../types/props"; defineProps<Props>();</script>', "export default {}", "ts", 'import type { Props } from "../types/props"; import type { Missing } from "missing-native-types"; defineProps<Props>();');
  const details = { script: "globalThis.result = 42;", scriptUrl: "vn://app.prod.js", defines: {}, engine: aotEngineIdentity("webkit.version=engine-test\nwebkit.asset=engine-aot.tar.gz\nwebkit.aot=1\nwebkit.sha256=" + "a".repeat(64)), sdkVersion: "test" };
  const path = archive.write(details);
  const first = readFileSync(path, "utf8");
  const manifest = JSON.parse(first);
  expect(manifest.entry).toBe("src/main.ts");
  expect(manifest.engine.jscPin).toBe("engine-test");
  expect(manifest.engine.asset).toBe("engine-aot.tar.gz");
  expect(manifest.engine.aot).toBe(true);
  expect(manifest.engine.sha256).toBe("a".repeat(64));
  expect(manifest.stage).toBe("typed-source-inputs");
  expect(manifest.files.map((input: any) => input.path)).toContain("types/only-type.d.ts");
  expect(manifest.files.map((input: any) => input.path)).toContain("types/props.d.ts");
  expect(manifest.files.map((input: any) => input.path)).toContain("src/ambient.d.ts");
  expect(manifest.files.map((input: any) => input.path)).toContain("tsconfig.base.json");
  expect(manifest.unresolvedImports.some((input: any) => input.specifier === "missing-native-types")).toBe(true);
  for (const input of manifest.files) {
    expect(input.path).not.toMatch(/^[a-z]:|^\//i);
    const bytes = readFileSync(join(out, input.blob));
    expect(hash(bytes)).toBe(input.sha256);
    expect(bytes.byteLength).toBe(input.bytes);
  }
  const retained = manifest.files.find((input: any) => input.path === "src/main.ts");
  writeFileSync(join(out, retained.blob), "stale or interrupted previous artifact");
  archive.write(details);
  expect(hash(readFileSync(join(out, retained.blob)))).toBe(retained.sha256);
  expect(readFileSync(path, "utf8")).toBe(first);
});


test("solution tsconfig retains referenced app project declarations and libraries", () => {
  const app = join(scratch, "solution-project");
  const out = join(scratch, "solution-archive");
  const entry = join(app, "src/main.ts");
  file(entry, "export const value: number = 1;");
  file(join(app, "src/globals.d.ts"), "declare const appOnly: number;");
  file(join(app, "tooling/tool.ts"), "export {};");
  file(join(app, "tsconfig.json"), '{"files":[],"references":[{"path":"./tsconfig.app.json"},{"path":"./tsconfig.node.json"}]}');
  file(join(app, "tsconfig.app.json"), '{"compilerOptions":{"types":[],"lib":["ES2023"]},"include":["src/**/*.ts"]}');
  file(join(app, "tsconfig.node.json"), '{"files":["tooling/tool.ts"]}');
  const archive = new AotInputs({ out, appRoot: app, entry, typescript: ts, aliases: {}, sourceRoots: { sdk: resolve(import.meta.dir, "..") } });
  archive.source(entry, readFileSync(entry, "utf8"));
  const path = archive.write({ script: "const value = 1;", scriptUrl: "vn://app.prod.js", defines: {}, engine: aotEngineIdentity(null), sdkVersion: "test" });
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  const paths = manifest.files.map((input: any) => input.path);
  expect(paths).toContain("tsconfig.app.json");
  expect(paths).toContain("tsconfig.node.json");
  expect(paths).toContain("src/globals.d.ts");
  expect(paths).not.toContain("tooling/tool.ts");
  expect(paths).toContain("@sdk/node_modules/typescript/lib/lib.es2023.d.ts");
});

test("real vn build emits typed Vue modules while VNPK keeps the same JavaScript", () => {
  const app = join(scratch, "app");
  file(join(app, "src/main.ts"), 'import { createApp } from "vue"; import App from "./App.vue"; createApp(App).mount("#app");');
  file(join(app, "src/App.vue"), '<script setup lang="ts">const count: number = 7;</script><template><div>{{ count }}</div></template>');
  file(join(app, "vn.config.json"), '{"name":"typed-aot-probe","version":"1.0.0"}');
  const cli = resolve(import.meta.dir, "../cli/vn.ts");
  const build = (out: string, inputs: boolean) => {
    const result = Bun.spawnSync([process.execPath, cli, "build", app, "--out", out, "--payload-only", "--json", ...(inputs ? ["--aot-inputs"] : [])], { stdout: "pipe", stderr: "pipe" });
    expect(result.exitCode, new TextDecoder().decode(result.stderr)).toBe(0);
    return JSON.parse(new TextDecoder().decode(result.stdout).trim().split("\n").pop()!);
  };
  const prepared = build(join(scratch, "prepared"), true);
  const normal = build(join(scratch, "normal"), false);
  expect(normal.aotInputs).toBeNull();
  const manifest = JSON.parse(readFileSync(prepared.aotInputs, "utf8"));
  expect(manifest.engine.asset).toBe("bun-webkit-windows-amd64-aot.tar.gz");
  expect(manifest.engine.aot).toBe(true);
  expect(manifest.engine.sha256).toMatch(/^[a-f0-9]{64}$/);
  const vue = manifest.files.find((input: any) => input.path === "src/App.vue" && input.kind === "vue-module");
  expect(vue.loader).toBe("ts");
  expect(readFileSync(join(dirname(prepared.aotInputs), vue.blob), "utf8")).toMatch(/count:\s*number/);
  expect(manifest.aliases.vue).toMatch(/^@sdk\/src\/runtime\.ts$/);
  const script = (path: string) => {
    const payload = readVnpk(readFileSync(path));
    return payload.bytes(payload.entries.find((input) => input.name === NAMES.SCRIPT)!);
  };
  const bytes = script(prepared.vnpk);
  expect(hash(bytes)).toBe(manifest.script.sha256);
  expect(bytes).toEqual(script(normal.vnpk));
}, 120_000);
