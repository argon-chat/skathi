// A Lottie or Telegram sticker referenced by URL is packed into the payload, as an image is.

import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { buildApp } from "../cli/build-app";
import { readPayload } from "../cli/payload";
import { KIND } from "../cli/skpk";

const FIXTURES = join(dirname(fileURLToPath(import.meta.url)), "fixtures");
const tmpDirs: string[] = [];
afterAll(() => {
  for (const dir of tmpDirs) rmSync(dir, { recursive: true, force: true });
});

test("sk build packs .tgs, .lottie and lottie .json files referenced by URL", async () => {
  const app = mkdtempSync(join(FIXTURES, "lottie-app-"));
  const out = mkdtempSync(join(tmpdir(), "sk-lottie-payload-"));
  tmpDirs.push(app, out);
  const files: Record<string, string> = {
    "sk.config.json": JSON.stringify({ name: "lottie-app", version: "1.0.0" }),
    "src/main.ts": [
      `import { createApp } from "vue";`,
      `import App from "./App.vue";`,
      `import dance from "./stickers/dance.lottie";`,
      `import jumpUrl from "./stickers/jump.json?url";`,
      `const wave = new URL("./stickers/wave.tgs", import.meta.url);`,
      `(globalThis as Record<string, unknown>).__lottie = { dance, jumpUrl, wave: wave.href };`,
      `createApp(App).mount("#app");`,
    ].join("\n"),
    "src/App.vue": `<template><lottie src="./stickers/duck.tgs" /><lottie src="./stickers/spin.json" /></template>`,
    "src/stickers/dance.lottie": "lottie dance",
    "src/stickers/jump.json": `{"v":"5.7.0","jump":true}`,
    "src/stickers/wave.tgs": "tgs wave",
    "src/stickers/duck.tgs": "tgs duck",
    "src/stickers/spin.json": `{"v":"5.7.0","spin":true}`,
  };
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(dirname(join(app, name)), { recursive: true });
    writeFileSync(join(app, name), text);
  }

  const result = await buildApp({ target: app, out, payloadOnly: true, log: () => {} });
  const payload = readPayload(readFileSync(result.skpk));
  for (const name of ["dance.lottie", "jump.json", "wave.tgs", "duck.tgs", "spin.json"]) {
    const key = `src/stickers/${name}`;
    const entry = payload.find(KIND.ASSET, key);
    expect(entry, key).toBeDefined();
    expect(Buffer.from(payload.bytes(entry!))).toEqual(readFileSync(join(app, key)));
  }
  const script = readFileSync(join(out, ".sk", "app.prod.js"), "utf8");
  expect(script).toContain(`"/src/stickers/wave.tgs"`);
  expect(script).not.toContain(`"spin":true`);
}, 60_000);
