import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { loadAppConfig } from "../cli/config";
import { readVnpk, NAMES } from "../cli/vnpk";

const root = mkdtempSync(join(tmpdir(), "vn-external-config-"));
const app = join(root, "client");
const external = join(root, "build", "app.json");
mkdirSync(join(app, "src"), { recursive: true });
mkdirSync(join(app, "assets"));
mkdirSync(join(app, "public"));
mkdirSync(join(root, "build"));
writeFileSync(join(app, "src", "main.ts"), "globalThis.__externalConfigProbe = __EXTERNAL_CONFIG_VALUE__; ");
writeFileSync(
  join(app, "vn.config.json"),
  JSON.stringify({ name: "tracked-config", version: "1.0.0", window: { title: "tracked-title" } }),
);
writeFileSync(join(app, "assets", "probe.txt"), "asset from app root");
writeFileSync(join(app, "public", "public.txt"), "public from app root");
writeFileSync(
  external,
  JSON.stringify({
    name: "external-config",
    version: "2.0.0",
    assets: ["assets"],
    publicDir: "public",
    define: { __EXTERNAL_CONFIG_VALUE__: '"override-value"' },
    window: { title: "external-title" },
  }),
);
afterAll(() => rmSync(root, { recursive: true, force: true }));

test("an external config replaces metadata while entry and relative assets retain the target root", () => {
  const config = loadAppConfig(app, { configPath: external });
  expect(config.name).toBe("external-config");
  expect(config.appRoot).toBe(app);
  expect(config.entry).toBe(join(app, "src", "main.ts"));
  expect(config.configPath).toBe(external);
  expect(config.assets).toEqual(["assets"]);
  expect(config.publicDir).toBe("public");
  expect(loadAppConfig(app).name).toBe("tracked-config");
  expect(() => loadAppConfig(app, { configPath: "" })).toThrow(/needs a path/);
  expect(() => loadAppConfig(app, { configPath: join(root, "missing.json") })).toThrow(/no such config/);
});

test("vn build propagates external window and define settings into the actual payload", () => {
  const out = join(root, "out");
  const cli = resolve(import.meta.dir, "../cli/vn.ts");
  const result = Bun.spawnSync(
    [process.execPath, cli, "build", app, "--config", external, "--out", out, "--payload-only", "--json"],
    { stdout: "pipe", stderr: "pipe" },
  );
  expect(result.exitCode, new TextDecoder().decode(result.stderr)).toBe(0);
  const summary = JSON.parse(new TextDecoder().decode(result.stdout).trim().split("\n").pop()!);
  expect(summary.name).toBe("external-config");
  const payload = readVnpk(new Uint8Array(readFileSync(summary.vnpk)));
  const text = (name: string) =>
    new TextDecoder().decode(payload.bytes(payload.entries.find((entry) => entry.name === name)!));
  expect(JSON.parse(text(NAMES.WINDOW)).title).toBe("external-title");
  expect(text(NAMES.SCRIPT)).toContain("override-value");
  expect(payload.entries.some((entry) => entry.name === "assets/probe.txt")).toBe(true);
  expect(payload.entries.some((entry) => entry.name === "public.txt")).toBe(true);
  expect(JSON.parse(readFileSync(join(app, "vn.config.json"), "utf8")).name).toBe("tracked-config");
}, 120_000);
