import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { modeVariant } from "../cli/build-app";
import { loadAppConfig } from "../cli/config";

const dirs: string[] = [];
afterAll(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true });
});

function config(body: Record<string, unknown>) {
  const dir = mkdtempSync(join(tmpdir(), "sk-modes-"));
  dirs.push(dir);
  writeFileSync(join(dir, "main.ts"), "");
  writeFileSync(join(dir, "sk.config.json"), JSON.stringify({ name: "m", entry: "main.ts", ...body }));
  return loadAppConfig(dir);
}

test("without a selection, build keeps devtools and publish drops them", () => {
  const c = config({});
  expect(modeVariant(c, "debug")).toBe("standard");
  expect(modeVariant(c, "release")).toBe("no-devtools");
  expect(modeVariant(c, undefined)).toBe("standard");
});

test("publish follows a selection that keeps devtools; build adds them to one that does not", () => {
  const keeps = config({ windows: { without: ["audio"] } });
  expect(modeVariant(keeps, "release")).toBe("no-audio");
  expect(modeVariant(keeps, "debug")).toBe("no-audio");
  const drops = config({ windows: { without: ["devtools", "webgpu"] } });
  expect(modeVariant(drops, "release")).toBe("no-devtools+no-webgpu");
  expect(modeVariant(drops, "debug")).toBe("no-webgpu");
});
