import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = mkdtempSync(join(here, "fixtures", "dev-asset-map-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

test("dev manifest maps external app and bundled npm asset URLs to canonical files", () => {
  const app = join(root, "app");
  const shared = join(root, "shared");
  const npm = join(root, "node_modules", "vn-asset-fixture");
  for (const dir of [join(app, "src"), shared, npm]) mkdirSync(dir, { recursive: true });
  writeFileSync(join(app, "vn.config.json"), "{}");
  writeFileSync(
    join(app, "package.json"),
    JSON.stringify({ private: true, dependencies: { "vn-asset-fixture": "1.0.0" } }),
  );
  writeFileSync(join(shared, "A Font[wght].ttf"), "direct font bytes");
  writeFileSync(join(npm, "font.ttf"), "npm font bytes");
  writeFileSync(
    join(npm, "package.json"),
    JSON.stringify({ name: "vn-asset-fixture", version: "1.0.0", main: "index.ts" }),
  );
  writeFileSync(join(npm, "index.ts"), 'import font from "./font.ttf?url"; export { font };');
  writeFileSync(
    join(app, "src", "main.ts"),
    'import direct from "../../shared/A Font[wght].ttf?url"; import {font} from "vn-asset-fixture"; globalThis.__fontUrls = [direct, font];',
  );
  const dist = join(root, "dist");
  const result = Bun.spawnSync(["bun", "build.ts", join(app, "src", "main.ts"), dist], { cwd: join(here, "..") });
  expect(result.exitCode, result.stdout.toString() + result.stderr.toString()).toBe(0);
  const manifest = JSON.parse(readFileSync(join(dist, "manifest.json"), "utf8"));
  const entries = Object.entries(manifest.assetFiles) as [string, string][];
  expect(entries).toHaveLength(2);
  expect(entries.every(([key]) => /^_vn\/fs\/[0-9a-f]+\//.test(key))).toBe(true);
  expect(entries.map(([, path]) => path).sort()).toEqual(
    [realpathSync(join(shared, "A Font[wght].ttf")), realpathSync(join(npm, "font.ttf"))].sort(),
  );
  expect(entries.map(([, path]) => readFileSync(path, "utf8")).sort()).toEqual(["direct font bytes", "npm font bytes"]);
});
