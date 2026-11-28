import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launch } from "../src/index";

test("web and native gamepad APIs run in the JIT host before any controller gesture", async () => {
  const profile = mkdtempSync(join(tmpdir(), "skathi-gamepad-"));
  try {
    const app = await launch({
      app: "packages/test/fixtures/gamepad/src/main.ts",
      label: "gamepad",
      env: { SK_DATA_DIR: profile },
    });
    try {
      await app.locator("#ready").waitFor();
      const result = JSON.parse(String(await app.eval("JSON.stringify(probeGamepads())")));
      expect(result).toEqual({ jit: true, native: true, deadZones: true, web: true, event: true, staleOutput: false });
    } finally {
      await app.close();
    }
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
}, 60_000);
