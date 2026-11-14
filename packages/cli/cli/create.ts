// `sk create [dir]`: a new app. In a terminal it asks for what the flags do not say: the directory, the
// name, the OSes to package for, the engine parts, whether published builds keep devtools, vue-router
// and pinia, and the package manager. `--yes` (or no terminal) takes the defaults.
//
// It writes package.json (the app root the builders resolve npm dependencies from), sk.config.json,
// src/, assets/, a generated 256² icon.png and a tsconfig.json that type-checks against the runtime.
// Nothing is overwritten: an existing file is an error, not a merge.

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import * as p from "@clack/prompts";
import { RUNTIME_SRC_DIR, runtimeRequire } from "../runtime-path";
import { ConfigError, OPTIONAL, parseParts, sanitizeExeName, type Part } from "./config";
import { PKG_DIR } from "./host";
import { encodePng } from "./icon";
import { currentPlatform, parsePlatform, type Platform } from "./platform";

export const PACKAGE_MANAGERS = ["bun", "npm", "pnpm", "yarn"] as const;
export type PackageManager = (typeof PACKAGE_MANAGERS)[number];

export interface CreateOptions {
  name?: string | undefined;
  platforms?: Platform[] | undefined;
  /** Engine parts left out of every build; devtools is {@link CreateOptions.keepDevtools}'s. */
  without?: Part[] | undefined;
  /** Whether `sk publish` keeps devtools (`sk build` always has them). */
  keepDevtools?: boolean | undefined;
  router?: boolean | undefined;
  packageManager?: PackageManager | undefined;
}

const ENGINE_PARTS: { value: Exclude<Part, "devtools">; label: string; hint: string }[] = [
  { value: "audio", label: "Audio", hint: "Web Audio, <audio>, sound from <video>" },
  { value: "clipboard", label: "Clipboard", hint: "navigator.clipboard" },
  { value: "storages", label: "Storage on disk", hint: "localStorage and IndexedDB that survive a restart" },
  { value: "indexeddb", label: "IndexedDB", hint: "indexedDB" },
  { value: "webgpu", label: "WebGPU", hint: "navigator.gpu" },
];

/** The package manager this was started from (`npm_config_user_agent`), else bun. */
export function detectPackageManager(agent = process.env.npm_config_user_agent ?? ""): PackageManager {
  return PACKAGE_MANAGERS.find((pm) => agent.startsWith(`${pm}/`)) ?? "bun";
}

export const runScript = (pm: PackageManager, script: string) =>
  pm === "npm" || pm === "bun" ? `${pm} run ${script}` : `${pm} ${script}`;

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

const json = (v: unknown) => JSON.stringify(v, null, 2) + "\n";

const STYLE = `
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
}`;

function sources(productName: string, router: boolean): Record<string, string> {
  if (!router) {
    return {
      "src/main.ts": `import { createApp } from "vue";
import App from "./App.vue";

createApp(App).mount();
`,
      "src/App.vue": `<script setup lang="ts">
import { onMounted, ref } from "vue";

const count = ref(0);
// A relative URL resolves against the app root in dev and against the payload once packaged.
const greeting = ref("…");
onMounted(async () => {
  const res = await fetch?.("assets/hello.txt");
  const text = res?.ok ? await res.text?.() : undefined;
  greeting.value = text?.trim() ?? "assets/hello.txt did not load";
});
</script>

<template>
  <div class="root">
    <div class="title">${productName}</div>
    <div class="line">{{ greeting }}</div>
    <div class="btn" @click="count++">clicked {{ count }}×</div>
  </div>
</template>

<style scoped>${STYLE}
</style>
`,
    };
  }
  return {
    "src/main.ts": `import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";

createApp(App).use(createPinia()).use(router).mount();
`,
    "src/router.ts": `import { createMemoryHistory, createRouter } from "vue-router";
import Home from "./views/Home.vue";
import About from "./views/About.vue";

export const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: "/", component: Home },
    { path: "/about", component: About },
  ],
});
`,
    "src/stores/counter.ts": `import { defineStore } from "pinia";
import { ref } from "vue";

export const useCounter = defineStore("counter", () => {
  const count = ref(0);
  const increment = () => count.value++;
  return { count, increment };
});
`,
    "src/App.vue": `<template>
  <div class="root">
    <div class="title">${productName}</div>
    <div class="nav">
      <RouterLink class="btn" to="/">Home</RouterLink>
      <RouterLink class="btn" to="/about">About</RouterLink>
    </div>
    <RouterView />
  </div>
</template>

<style scoped>${STYLE}
.nav {
  display: flex;
  gap: 8px;
}
</style>
`,
    "src/views/Home.vue": `<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useCounter } from "../stores/counter";

const counter = useCounter();
// A relative URL resolves against the app root in dev and against the payload once packaged.
const greeting = ref("…");
onMounted(async () => {
  const res = await fetch?.("assets/hello.txt");
  const text = res?.ok ? await res.text?.() : undefined;
  greeting.value = text?.trim() ?? "assets/hello.txt did not load";
});
</script>

<template>
  <div>{{ greeting }}</div>
  <div class="btn" @click="counter.increment()">clicked {{ counter.count }}×</div>
</template>

<style scoped>
.btn {
  align-self: flex-start;
  padding: 6px 14px;
  border-radius: 8px;
  background: #ffffff22;
}
</style>
`,
    "src/views/About.vue": `<script setup lang="ts">
import { useCounter } from "../stores/counter";

const counter = useCounter();
</script>

<template>
  <div>The counter lives in a pinia store: {{ counter.count }}, here too.</div>
</template>
`,
  };
}

/** Writes the app; every option has a default. Returns the files written. */
export function create(target: string, options: CreateOptions = {}): string[] {
  const dir = resolve(target);
  const name = (options.name ?? basename(dir)).trim();
  if (!name) throw new ConfigError("sk create: the app needs a name");
  const productName = name
    .replace(/(^|[-_.\s])(\w)/g, (_, sep: string, c: string) => (sep ? " " : "") + c.toUpperCase())
    .trim();
  const exeName = sanitizeExeName(productName) || "App";
  const slug =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "app";
  const platforms = options.platforms?.length ? options.platforms : [currentPlatform()];
  const without: Part[] = OPTIONAL.filter((part) =>
    part === "devtools" ? !options.keepDevtools : (options.without ?? []).includes(part),
  );
  const rel = (to: string) => relative(dir, to).split(sep).join("/");
  const runtime = rel(RUNTIME_SRC_DIR);
  const pkg = (name: string) => rel(dirname(runtimeRequire.resolve(`${name}/package.json`)));
  const apps = rel(join(PKG_DIR, "..", "..", "apps"));

  const files: Record<string, string | Uint8Array> = {
    "package.json": json({
      name: slug,
      version: "0.1.0",
      private: true,
      type: "module",
      scripts: { dev: "sk dev", build: "sk build", publish: "sk publish" },
    }),
    "sk.config.json": json({
      name,
      productName,
      version: "0.1.0",
      platforms,
      window: { title: productName, width: 900, height: 640, backdrop: "mica" },
      assets: ["assets"],
      windows: { exeName, icon: "icon.png", without },
      ...(platforms.includes("macos") ? { macos: { bundleId: `com.example.${slug}` } } : {}),
    }),
    ...sources(productName, options.router ?? false),
    "assets/hello.txt": `hello from ${name}'s assets\n`,
    "icon.png": icon(),
    "tsconfig.json": json({
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
          "@skathi-native/runtime": [`${runtime}/runtime.ts`],
          "vue-router": [pkg("vue-router")],
          pinia: [pkg("pinia")],
        },
      },
      // Template types from the runtime: an installed `vue` package brings DOM typings the engine does not have.
      vueCompilerOptions: { lib: "@skathi-native/runtime" },
      include: [
        `${runtime}/engine.d.ts`,
        `${runtime}/protocol.d.ts`,
        `${runtime}/indexeddb.d.ts`,
        `${apps}/env.d.ts`,
        "src/**/*.ts",
        "src/**/*.vue",
      ],
    }),
  };

  for (const rel of Object.keys(files)) {
    if (existsSync(join(dir, rel)))
      throw new ConfigError(`sk create: ${join(dir, rel)} already exists — nothing was written`);
  }
  const written: string[] = [];
  for (const [rel, body] of Object.entries(files)) {
    const path = join(dir, rel);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, body);
    written.push(path);
  }
  return written;
}

export interface CreateFlags {
  name?: string | undefined;
  os?: string | undefined;
  without?: string | undefined;
  devtools?: boolean | undefined;
  router?: boolean | undefined;
  pm?: string | undefined;
  yes?: boolean | undefined;
}

function parsePm(v: string): PackageManager {
  const pm = PACKAGE_MANAGERS.find((m) => m === v.trim().toLowerCase());
  if (!pm) throw new ConfigError(`--pm ${JSON.stringify(v)}: expected one of ${PACKAGE_MANAGERS.join(", ")}`);
  return pm;
}

function answered<T>(value: T): Exclude<T, symbol> {
  if (p.isCancel(value) || typeof value === "symbol") {
    p.cancel("Nothing was created.");
    process.exit(130);
  }
  return value as Exclude<T, symbol>;
}

/** `sk create`: the questions the flags leave open, then {@link create}. */
export async function createCommand(target: string | undefined, flags: CreateFlags): Promise<number> {
  const ask = !flags.yes && process.stdin.isTTY === true && process.stdout.isTTY === true;
  const options: CreateOptions = {
    name: flags.name,
    platforms: flags.os
      ?.split(/[\s,]+/)
      .filter(Boolean)
      .map(parsePlatform),
    without: flags.without !== undefined ? parseParts(flags.without, "--without") : undefined,
    keepDevtools: flags.devtools,
    router: flags.router,
    packageManager: flags.pm !== undefined ? parsePm(flags.pm) : undefined,
  };
  let dir = target;

  if (ask) {
    p.intro("sk create");
    dir ??= answered(
      await p.text({
        message: "Where should the app go?",
        placeholder: "my-app",
        defaultValue: "my-app",
        validate: (v) =>
          v && existsSync(join(resolve(v), "package.json")) ? `${v} already has a package.json` : undefined,
      }),
    );
    options.name ??= answered(
      await p.text({
        message: "App name",
        initialValue: basename(resolve(dir)),
        validate: (v) => (v?.trim() ? undefined : "it needs a name"),
      }),
    );
    options.platforms ??= answered(
      await p.multiselect<Platform>({
        message: "Package it for",
        options: [
          { value: "windows", label: "Windows" },
          { value: "macos", label: "macOS" },
        ],
        initialValues: [currentPlatform()],
        required: true,
      }),
    );
    if (!options.without) {
      const kept = answered(
        await p.multiselect<Exclude<Part, "devtools">>({
          message: "Engine parts (leaving one out makes the exe smaller)",
          options: ENGINE_PARTS,
          initialValues: ENGINE_PARTS.map((e) => e.value),
          required: false,
        }),
      );
      options.without = ENGINE_PARTS.map((e) => e.value).filter((v) => !kept.includes(v));
    }
    options.keepDevtools ??= answered(
      await p.confirm({
        message: "Keep devtools (F12) in published builds? sk build always has them",
        initialValue: false,
      }),
    );
    options.router ??= answered(await p.confirm({ message: "Add vue-router and pinia?", initialValue: true }));
    options.packageManager ??= answered(
      await p.select<PackageManager>({
        message: "Package manager",
        options: PACKAGE_MANAGERS.map((pm) => ({ value: pm, label: pm })),
        initialValue: detectPackageManager(),
      }),
    );
  }

  dir ??= "my-app";
  const pm = options.packageManager ?? detectPackageManager();
  create(dir, options);
  const where = relative(process.cwd(), resolve(dir)) || ".";
  const next = [...(where === "." ? [] : [`cd ${where}`]), runScript(pm, "dev")].join("\n");
  if (ask) {
    p.note(next, "Next");
    p.outro(`${options.name ?? basename(resolve(dir))} is ready`);
  } else {
    console.log(`[sk] created ${resolve(dir)}\n  next: ${next.replace(/\n/g, " && ")}`);
  }
  return 0;
}
