// packages/runtime/sk.config.schema.json is written by hand; these hold it to the keys loadAppConfig knows.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { loadAppConfig, MACOS_KEYS, PACK_KEYS, TOP_LEVEL, WINDOWS_KEYS } from "../cli/config";
import { CONFIG_SCHEMA, create } from "../cli/create";

interface Schema {
  type?: string;
  properties?: Record<string, Schema>;
  items?: Schema;
  additionalProperties?: boolean | Schema;
  patternProperties?: Record<string, Schema>;
  description?: string;
  $ref?: string;
  definitions?: Record<string, Schema>;
}

const SCHEMA_PATH = resolve(import.meta.dir, "../../runtime/sk.config.schema.json");
const schema = JSON.parse(readFileSync(SCHEMA_PATH, "utf8")) as Schema;
const props = (s: Schema | undefined): string[] => Object.keys(s?.properties ?? {}).sort();
const sorted = (keys: Iterable<string>) => [...keys].sort();

const dirs: string[] = [];
afterAll(() => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), "sk-schema-"));
  dirs.push(d);
  return d;
};

describe("the schema and the loader know the same keys", () => {
  test("top level", () => {
    expect(props(schema).filter((k) => k !== "$schema")).toEqual(sorted(TOP_LEVEL));
  });

  test("windows, macos and a pack, where unknown keys are errors", () => {
    const windows = schema.properties?.windows;
    const macos = schema.properties?.macos;
    const pack = schema.properties?.packs?.items;
    expect(props(windows)).toEqual(sorted(WINDOWS_KEYS));
    expect(props(macos)).toEqual(sorted(MACOS_KEYS));
    expect(props(pack)).toEqual(sorted(PACK_KEYS));
    for (const s of [windows, macos, pack]) {
      expect(s?.additionalProperties).toBe(false);
      expect(Object.keys(s?.patternProperties ?? {})).toEqual(["^\\$"]);
    }
  });

  test("every key has a description", () => {
    const missing: string[] = [];
    const visit = (s: Schema, path: string) => {
      for (const [key, child] of Object.entries(s.properties ?? {})) {
        const where = `${path}${key}`;
        const target = child.$ref ? schema.definitions?.[child.$ref.replace("#/definitions/", "")] : child;
        if (!child.description && !target?.description) missing.push(where);
        visit(child, `${where}.`);
        if (child.items) visit(child.items, `${where}[].`);
      }
    };
    visit(schema, "");
    expect(missing).toEqual([]);
  });
});

test("the schema and the CSS custom data ship in @skathi-native/runtime", () => {
  const runtime = resolve(import.meta.dir, "../../runtime");
  const pkg = JSON.parse(readFileSync(join(runtime, "package.json"), "utf8")) as {
    files: string[];
    exports: Record<string, string>;
  };
  for (const file of ["sk.config.schema.json", "css-data.json"]) {
    expect(pkg.files).toContain(file);
    expect(pkg.exports[`./${file}`]).toBe(`./${file}`);
  }
  const css = JSON.parse(readFileSync(join(runtime, "css-data.json"), "utf8")) as {
    version: number;
    properties: { name: string; description: string; values?: { name: string; description: string }[] }[];
  };
  expect(css.version).toBe(1.1);
  for (const p of css.properties) {
    expect(p.description.length).toBeGreaterThan(0);
    for (const v of p.values ?? []) expect(v.description.length).toBeGreaterThan(0);
  }
});

describe("$schema", () => {
  test("the loader takes it without a warning", () => {
    const dir = tmp();
    writeFileSync(join(dir, "main.ts"), "");
    writeFileSync(
      join(dir, "sk.config.json"),
      JSON.stringify({ $schema: CONFIG_SCHEMA, name: "s", version: "1.0.0", windows: { icon: "main.ts" } }),
    );
    expect(loadAppConfig(dir).warnings).toEqual([]);
  });

  test("sk create writes it first, and only keys the schema knows", () => {
    const dir = join(tmp(), "app");
    create(dir, { platforms: ["windows", "macos"] });
    const config = JSON.parse(readFileSync(join(dir, "sk.config.json"), "utf8")) as Record<string, unknown>;
    expect(Object.keys(config)[0]).toBe("$schema");
    expect(config.$schema).toBe(CONFIG_SCHEMA);
    expect(CONFIG_SCHEMA.endsWith("/@skathi-native/runtime/sk.config.schema.json")).toBe(true);
    for (const key of Object.keys(config)) expect(props(schema)).toContain(key);
    for (const block of ["windows", "macos"] as const) {
      for (const key of Object.keys(config[block] as object)) expect(props(schema.properties?.[block])).toContain(key);
    }
  });
});
