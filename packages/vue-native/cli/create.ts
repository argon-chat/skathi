// `vn create <dir>` — the smallest app `vn build` turns into an exe: vn.config.json, src/main.ts,
// src/App.vue (a counter and a fetch), assets/ (what gets packaged), a generated 256² icon.png so
// the first exe has one, and a tsconfig.json that type-checks against the runtime, not lib.dom.
// Nothing is overwritten: an existing file is an error, not a merge.

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { basename, join, relative, resolve, sep } from "node:path";
import { sanitizeExeName } from "./config";
import { PKG_DIR } from "./host";
import { encodePng } from "./icon";

function icon(): Uint8Array {
  const size = 256;
  const data = new Uint8Array(size * size * 4);
  const r = 56;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dx = Math.max(0, Math.abs(x - 127.5) - (128 - r));
      const dy = Math.max(0, Math.abs(y - 127.5) - (128 - r));
      const inside = dx * dx + dy * dy <= r * r;
      const t = (x + y) / (2 * size);
      const o = (y * size + x) * 4;
      data[o] = Math.round(0x42 + (0x2f - 0x42) * t);
      data[o + 1] = Math.round(0xb8 + (0x6f - 0xb8) * t);
      data[o + 2] = Math.round(0x83 + (0xd8 - 0x83) * t);
      data[o + 3] = inside ? 255 : 0;
    }
  }
  return encodePng({ width: size, height: size, data });
}

export function create(target: string, options: { name?: string } = {}): string[] {
  const dir = resolve(target);
  const name = (options.name ?? basename(dir)).trim();
  if (!name) throw new Error("vn create: the app needs a name");
  const productName = name.replace(/(^|[-_.\s])(\w)/g, (_, sep: string, c: string) => (sep ? " " : "") + c.toUpperCase()).trim();
  const exeName = sanitizeExeName(productName) || "App";
  const runtime = relative(dir, join(PKG_DIR, "src")).split(sep).join("/");
  const apps = relative(dir, join(PKG_DIR, "..", "..", "apps")).split(sep).join("/");

  const files: Record<string, string | Uint8Array> = {
    "vn.config.json":
      JSON.stringify(
        {
          name,
          productName,
          version: "0.1.0",
          window: { title: productName, width: 900, height: 640, backdrop: "mica" },
          assets: ["assets"],
          windows: { exeName, icon: "icon.png" },
        },
        null,
        2,
      ) + "\n",
    "src/main.ts": `import { createApp } from "vue";
import App from "./App.vue";

createApp(App).mount();
`,
    "src/App.vue": `<script setup lang="ts">
import { onMounted, ref } from "vue";

const count = ref(0);
// A relative URL resolves against the app root (apps/<app>/ in dev, the payload once packaged).
const greeting = ref("…");
onMounted(async () => {
  const res = await fetch("assets/hello.txt");
  greeting.value = res.ok ? (await res.text()).trim() : \`HTTP \${res.status}\`;
});
</script>

<template>
  <div class="root">
    <div class="title">${productName}</div>
    <div class="line">{{ greeting }}</div>
    <div class="btn" @click="count++">clicked {{ count }}×</div>
  </div>
</template>

<style scoped>
.root {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 24px;
  color: white;
}
.title {
  font-size: 22px;
  font-weight: 600;
}
.btn {
  align-self: flex-start;
  padding: 6px 14px;
  border-radius: 8px;
  background: #ffffff22;
}
</style>
`,
    "assets/hello.txt": `hello from ${name}'s assets\n`,
    "icon.png": icon(),
    "tsconfig.json":
      JSON.stringify(
        {
          compilerOptions: {
            target: "ESNext",
            module: "ESNext",
            moduleResolution: "bundler",
            lib: ["ESNext"],
            strict: true,
            noEmit: true,
            skipLibCheck: true,
            useDefineForClassFields: false,
            isolatedModules: true,
            types: [],
            baseUrl: ".",
            paths: {
              vue: [`${runtime}/runtime.ts`],
              "vue-native": [`${runtime}/runtime.ts`],
            },
          },
          include: [
            `${runtime}/engine.d.ts`,
            `${runtime}/protocol.d.ts`,
            `${runtime}/indexeddb.d.ts`,
            `${apps}/env.d.ts`,
            "src/**/*.ts",
            "src/**/*.vue",
          ],
        },
        null,
        2,
      ) + "\n",
  };

  const written: string[] = [];
  for (const rel of Object.keys(files)) {
    if (existsSync(join(dir, rel))) throw new Error(`vn create: ${join(dir, rel)} already exists — nothing was written`);
  }
  for (const [rel, body] of Object.entries(files)) {
    const path = join(dir, rel);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, body);
    written.push(path);
  }
  console.log(`[vn] created ${name} in ${dir}\n  next: bun ${relative(process.cwd(), join(PKG_DIR, "cli", "vn.ts"))} build ${relative(process.cwd(), dir) || "."}`);
  return written;
}
