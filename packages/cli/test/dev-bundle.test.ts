import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadAppConfig } from "../cli/config";
import { create } from "../cli/create";
import { devBundle } from "../cli/dev-bundle";

const root = mkdtempSync(join(tmpdir(), "sk-dev-bundle-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

test("the dev app carries the usage descriptions under its own bundle id, and copies the host only when it changed", () => {
  const app = join(root, "my-app");
  create(app, { platforms: ["macos"] });
  const host = join(root, "sk-app");
  writeFileSync(host, "host v1");
  const config = loadAppConfig(app);

  const first = devBundle({ config, host });
  expect(first.changed).toBe(true);
  expect(first.exe).toBe(join(first.app, "Contents", "MacOS", "sk-app"));
  expect(readFileSync(first.exe, "utf8")).toBe("host v1");
  const plist = readFileSync(join(first.app, "Contents", "Info.plist"), "utf8");
  expect(plist).toContain("<key>NSMicrophoneUsageDescription</key>");
  expect(plist).toContain("<key>NSCameraUsageDescription</key>");
  expect(plist).toContain("<string>com.example.my-app.dev</string>");
  expect(plist).toContain("<key>CFBundleExecutable</key>\n\t<string>sk-app</string>");

  expect(devBundle({ config, host }).changed).toBe(false);

  writeFileSync(host, "host v2");
  utimesSync(host, new Date(), new Date(Date.now() + 5000));
  const rebuilt = devBundle({ config, host });
  expect(rebuilt.changed).toBe(true);
  expect(readFileSync(rebuilt.exe, "utf8")).toBe("host v2");
});
