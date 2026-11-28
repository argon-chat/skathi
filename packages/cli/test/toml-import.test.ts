import { createLoader } from "../../runtime/test/loader-harness";
import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const CLI = resolve(import.meta.dir, "..");
const root = mkdtempSync(join(import.meta.dir, "fixtures", "toml-import-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

const SETTINGS = `title = "Skathi"
big = 9007199254740993
when = 1979-05-27T07:32:00Z

[window]
size = [800, 0x7FFF_FFFF_FFFF_FFFF, -9223372036854775808]
`;
const BROKEN = `big = 9007199254740993

  name "x"
`;

function app(name: string, settings: string): string {
  const dir = join(root, name);
  const files: Record<string, string> = {
    "sk.config.json": "{}",
    "src/main.ts": `import settings from "./config/settings.toml";\n(globalThis as Record<string, unknown>).__settings = settings;\n`,
    "src/config/settings.toml": settings,
  };
  for (const [file, text] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, file)), { recursive: true });
    writeFileSync(join(dir, file), text);
  }
  return dir;
}

const run = (...args: string[]) => Bun.spawnSync([process.execPath, ...args], { cwd: CLI });
const output = (r: ReturnType<typeof run>) => r.stdout.toString() + r.stderr.toString();
const parse = (text: string) => ({ parsed: text });

test("sk build: a .toml import is TOML.parse of the file's text at run time", () => {
  const dir = app("prod", SETTINGS);
  const out = join(dir, "dist", "app.prod.js");
  const result = run("prod.ts", join(dir, "src", "main.ts"), out);
  expect(result.exitCode, output(result)).toBe(0);
  const globals: Record<string, unknown> = {};
  new Function("globalThis", "__sk_std", readFileSync(out, "utf8"))(globals, { TOML: { parse } });
  expect(globals.__settings).toEqual({ parsed: SETTINGS });
});

test("sk dev: a .toml import is built into its importer and parsed by skathi:toml", async () => {
  const dir = app("dev", SETTINGS);
  const dist = join(dir, "dev");
  const result = run("build.ts", join(dir, "src", "main.ts"), dist);
  expect(result.exitCode, output(result)).toBe(0);
  const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8")) as { modules: unknown };
  expect(JSON.stringify(manifest.modules)).not.toContain("settings.toml");
  const { SK, sandbox } = createLoader({ toml: { TOML: { parse } } });
  const main = readdirSync(join(dist, "modules")).find((name) => name.endsWith("main.ts.js"))!;
  new Function("__SK__", readFileSync(join(dist, "modules", main), "utf8"))(SK);
  await SK.boot(JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8")).entry);
  expect((sandbox as Record<string, unknown>).__settings).toEqual({ parsed: SETTINGS });
  expect(SK.loaded("skathi:toml")).toBe(true);
});

test.each([
  ["prod.ts", "dist/app.prod.js"],
  ["build.ts", "dev"],
])("%s fails on invalid TOML at the file, line and column", (builder, out) => {
  const dir = app(`broken-${builder}`, BROKEN);
  const result = run(builder, join(dir, "src", "main.ts"), join(dir, out));
  expect(result.exitCode).not.toBe(0);
  expect(output(result)).toContain(`settings.toml:3:8`);
});
