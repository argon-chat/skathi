// `sk create [dir]`: a new app. In a terminal it asks for what the flags do not say: the directory, the
// name, the template, the OSes to package for, whether published builds keep devtools,
// and the package manager. `--yes` (or no terminal) takes the defaults.
//
// It copies the template (templates/<template>/ here, or the gallery repo's) with the placeholders filled
// in, and writes package.json (the app root the builders resolve npm dependencies from), sk.config.json, icon.png (the
// template's own, else the Skathi icon) and a tsconfig.json that type-checks against the runtime.
// Nothing is overwritten: an existing file is an error, not a merge.

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, extname, join, relative, resolve } from "node:path";
import cliPackage from "../package.json";
import * as p from "./prompt";
import { runtimeRequire } from "../runtime-path";
import { ConfigError, sanitizeExeName, VITE_CONFIG_FILES } from "./config";
import { fetchGallery, galleryDir } from "./gallery";
import { CLI_VERSION } from "./host";
import { currentPlatform, parsePlatform, type Platform } from "./platform";
import { colorEnabled, paint } from "./term";

export const PACKAGE_MANAGERS = ["bun", "npm", "pnpm", "yarn"] as const;
export type PackageManager = (typeof PACKAGE_MANAGERS)[number];

export const TEMPLATES = [
  "showcase",
  "minimal",
  "svelte",
  "player",
  "weather",
  "draw",
  "chip8",
  "sequencer",
  "rv32",
] as const;
export type Template = (typeof TEMPLATES)[number];

export const FRAMEWORKS = ["vue", "svelte"] as const;
export type Framework = (typeof FRAMEWORKS)[number];

/** The templates `sk create` offers: `svelte` is `minimal` with the Svelte answer. */
const OFFERED: readonly Template[] = TEMPLATES.filter((t) => t !== "svelte");

interface Checker {
  script: string;
  devDependencies: Record<string, string>;
  /** The component files the app's tsconfig.json includes besides its .ts sources. */
  sources: string;
  /** The runtime's tsconfig the app's extends. */
  tsconfig: string;
}

const VUE_TSC: Checker = {
  script: "vue-tsc --noEmit",
  devDependencies: { "vue-tsc": "^3" },
  sources: "src/**/*.vue",
  tsconfig: "@skathi-native/runtime/tsconfig.app.json",
};
const SVELTE_CHECK: Checker = {
  // Styles are `sk check`'s to judge: svelte-check does not know the properties Skathi adds.
  script: "svelte-check --diagnostic-sources js,svelte",
  devDependencies: { "svelte-check": "^4" },
  sources: "src/**/*.svelte",
  tsconfig: "@skathi-native/runtime/tsconfig.svelte.json",
};

const VITE_PLUGINS = {
  vue: { "@vitejs/plugin-vue": cliPackage.devDependencies["@vitejs/plugin-vue"] },
  svelte: { "@sveltejs/vite-plugin-svelte": "^7.1.2" },
};

/** The versions `sk create` gives a template's framework packages: vue and svelte as the runtime accepts them. */
const FRAMEWORK_VERSIONS: Record<string, string> = {
  ...(runtimeRequire("./package.json") as { peerDependencies?: Record<string, string> }).peerDependencies,
  pinia: "^4.0.3",
  "vue-router": "^4.6.4",
};

interface TemplateInfo {
  hint: string;
  framework: Framework;
  dependencies: string[];
  window: Record<string, unknown>;
  packs?: { name: string; match?: string[] }[];
  /** package.json scripts beside the usual ones. */
  scripts?: Record<string, string>;
  /** .gitignore lines beside the usual ones: what an install fetches. */
  ignore?: string[];
  /** sk.config.json `macos` keys beside `bundleId`: the usage descriptions of the devices the template asks for. */
  macos?: Record<string, string>;
  checker: Checker;
}

const TEMPLATE_INFO: Record<Template, TemplateInfo> = {
  showcase: {
    hint: "every Skathi feature, one page each",
    framework: "vue",
    dependencies: ["vue", "pinia", "vue-router"],
    window: { width: 1100, height: 720, minWidth: 480, minHeight: 360, titleBarStyle: "hidden", titleBarHeight: 40 },
    packs: [{ name: "media", match: ["assets/media/**"] }],
    macos: {
      microphoneUsageDescription: "The Permissions page records from the microphone when you ask it to.",
      cameraUsageDescription: "The Permissions page asks for the camera when you press Ask.",
    },
    checker: VUE_TSC,
  },
  minimal: {
    hint: "one window, one component, in Vue or Svelte",
    framework: "vue",
    dependencies: ["vue"],
    window: { width: 760, height: 520, titleBarStyle: "default" },
    checker: VUE_TSC,
  },
  svelte: {
    hint: "the minimal app in Svelte 5",
    framework: "svelte",
    dependencies: ["svelte"],
    window: { width: 760, height: 520, titleBarStyle: "default" },
    checker: SVELTE_CHECK,
  },
  player: {
    hint: "a music player in Svelte 5, dressed in shaders",
    framework: "svelte",
    dependencies: ["svelte"],
    window: { width: 1240, height: 800, minWidth: 480, minHeight: 600, titleBarStyle: "hidden", titleBarHeight: 44 },
    packs: [{ name: "media", match: ["assets/music/**", "assets/covers/**"] }],
    checker: SVELTE_CHECK,
  },
  weather: {
    hint: "a weather app in Vue, its sky and rain drawn by shaders",
    framework: "vue",
    dependencies: ["vue"],
    window: { width: 1180, height: 880, minWidth: 980, minHeight: 780, titleBarStyle: "hidden", titleBarHeight: 72 },
    packs: [{ name: "textures", match: ["assets/textures/**"] }],
    checker: VUE_TSC,
  },
  draw: {
    hint: "an infinite canvas for pens in Vue, drawn with WebGPU",
    framework: "vue",
    dependencies: ["vue"],
    window: {
      width: 1280,
      height: 820,
      minWidth: 480,
      minHeight: 420,
      titleBarStyle: "hidden",
      titleBarHeight: 40,
      backdrop: "none",
    },
    checker: VUE_TSC,
  },
  sequencer: {
    hint: "a drum machine in Svelte 5, its dials, keys and meter drawn by shaders",
    framework: "svelte",
    dependencies: ["svelte"],
    window: { width: 1200, height: 820, minWidth: 1080, minHeight: 760, titleBarStyle: "hidden", titleBarHeight: 40 },
    checker: SVELTE_CHECK,
  },
  chip8: {
    hint: "a CHIP-8 emulator in Vue, with a debugger beside the screen",
    framework: "vue",
    dependencies: ["vue"],
    window: { width: 1120, height: 800, minWidth: 900, minHeight: 700, titleBarStyle: "default" },
    checker: VUE_TSC,
  },
  rv32: {
    hint: "a RISC-V emulator in Vue with a debugger, running the Doom engine as its guest",
    framework: "vue",
    dependencies: ["vue"],
    window: { width: 1200, height: 880, minWidth: 1040, minHeight: 760, titleBarStyle: "hidden", titleBarHeight: 40 },
    packs: [{ name: "guest", match: ["assets/**"] }],
    scripts: { postinstall: "bun scripts/fetch-wad.mjs", wad: "bun scripts/fetch-wad.mjs" },
    ignore: ["assets/doom1.wad"],
    checker: VUE_TSC,
  },
};

export const TEMPLATES_DIR = resolve(import.meta.dir, "..", "templates");

/** Templates kept in the skathi-templates repo rather than in this package. */
const GALLERY: ReadonlySet<Template> = new Set(["player", "weather", "draw", "chip8", "sequencer", "rv32"]);

function templateDir(template: Template): string {
  if (!GALLERY.has(template)) return join(TEMPLATES_DIR, template);
  const dir = galleryDir(template);
  if (!dir) throw new ConfigError(`sk create: the ${template} template has not been downloaded`);
  return dir;
}

/** Downloads the template first when it lives in the gallery repo. */
export async function prepareTemplate(template: Template): Promise<void> {
  if (GALLERY.has(template)) await fetchGallery(template);
}

export const CONFIG_SCHEMA = "./node_modules/@skathi-native/runtime/sk.config.schema.json";

/** Files whose text has its `{{key}}` placeholders filled in; anything else is copied byte for byte. */
const TEXT_FILES = new Set([".vue", ".svelte", ".ts", ".css", ".txt", ".json", ".html", ".svg"]);

export interface CreateOptions {
  name?: string | undefined;
  template?: Template | undefined;
  platforms?: Platform[] | undefined;
  /** Whether `sk publish` keeps devtools (`sk build` always has them). */
  keepDevtools?: boolean | undefined;
  packageManager?: PackageManager | undefined;
}

/** The package manager this was started from (`npm_config_user_agent`), else bun. */
export function detectPackageManager(agent = process.env.npm_config_user_agent ?? ""): PackageManager {
  return PACKAGE_MANAGERS.find((pm) => agent.startsWith(`${pm}/`)) ?? "bun";
}

export const runScript = (pm: PackageManager, script: string) =>
  pm === "npm" || pm === "bun" ? `${pm} run ${script}` : `${pm} ${script}`;

const json = (v: unknown) => JSON.stringify(v, null, 2) + "\n";

function listFiles(dir: string, prefix = ""): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    return entry.isDirectory() ? listFiles(join(dir, entry.name), rel) : [rel];
  });
}

/** The template's files, keyed by app-relative path; `{{key}}` is replaced in text files, escaped for a string literal except in .txt. */
export function templateFiles(template: Template, values: Record<string, string>): Record<string, string | Uint8Array> {
  const dir = templateDir(template);
  const out: Record<string, string | Uint8Array> = {};
  for (const rel of listFiles(dir)) {
    const bytes = readFileSync(join(dir, rel));
    const ext = extname(rel);
    if (!TEXT_FILES.has(ext)) {
      out[rel] = bytes;
      continue;
    }
    out[rel] = bytes.toString("utf8").replace(/\{\{(\w+)\}\}/g, (whole, key: string) => {
      const value = values[key];
      if (value === undefined) return whole;
      return ext === ".txt" ? value : JSON.stringify(value).slice(1, -1);
    });
  }
  return out;
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
  const template = options.template ?? "showcase";
  const info = TEMPLATE_INFO[template];
  const platforms = options.platforms?.length ? options.platforms : [currentPlatform()];
  const sdk = `^${CLI_VERSION}`;

  const templateContents = templateFiles(template, { name, productName });
  const hasViteConfig = VITE_CONFIG_FILES.some((name) => Object.hasOwn(templateContents, name));
  const files: Record<string, string | Uint8Array> = {
    "icon.png": readFileSync(join(TEMPLATES_DIR, "icon.png")),
    ...templateContents,
    ...(!hasViteConfig && {
      "vite.config.ts": readFileSync(join(TEMPLATES_DIR, "config", info.framework, "vite.config.ts"), "utf8"),
    }),
    "package.json": json({
      name: slug,
      version: "0.1.0",
      private: true,
      type: "module",
      scripts: {
        dev: "sk dev",
        build: "sk build",
        publish: "sk publish",
        check: "sk check",
        typecheck: info.checker.script,
        ...info.scripts,
      },
      dependencies: {
        "@skathi-native/runtime": sdk,
        ...Object.fromEntries(info.dependencies.map((dep) => [dep, FRAMEWORK_VERSIONS[dep]])),
      },
      devDependencies: {
        "@skathi-native/cli": sdk,
        "vite-plus": cliPackage.peerDependencies["vite-plus"],
        ...VITE_PLUGINS[info.framework],
        typescript: "^6",
        ...info.checker.devDependencies,
      },
    }),
    "sk.config.json": json({
      $schema: CONFIG_SCHEMA,
      name,
      productName,
      version: "0.1.0",
      platforms,
      publishDevtools: options.keepDevtools === true,
      window: { title: productName, backdrop: "mica", ...info.window },
      assets: ["assets"],
      ...(info.packs ? { packs: info.packs } : {}),
      windows: { exeName, icon: "icon.png" },
      ...(platforms.includes("macos") ? { macos: { bundleId: `com.example.${slug}`, ...info.macos } } : {}),
    }),
    "tsconfig.json": json({
      extends: info.checker.tsconfig,
      include: ["src/**/*.ts", info.checker.sources],
    }),
    ".gitignore": ["node_modules/", "dist/", ...(info.ignore ?? []), ""].join("\n"),
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
  template?: string | undefined;
  framework?: string | undefined;
  os?: string | undefined;
  devtools?: boolean | undefined;
  pm?: string | undefined;
  yes?: boolean | undefined;
  color?: boolean | undefined;
}

function parsePm(v: string): PackageManager {
  const pm = PACKAGE_MANAGERS.find((m) => m === v.trim().toLowerCase());
  if (!pm) throw new ConfigError(`--pm ${JSON.stringify(v)}: expected one of ${PACKAGE_MANAGERS.join(", ")}`);
  return pm;
}

export function parseTemplate(v: string): Template {
  const template = TEMPLATES.find((t) => t === v.trim().toLowerCase());
  if (!template) throw new ConfigError(`--template ${JSON.stringify(v)}: expected one of ${TEMPLATES.join(", ")}`);
  return template;
}

function parseFramework(v: string): Framework {
  const framework = FRAMEWORKS.find((f) => f === v.trim().toLowerCase());
  if (!framework) throw new ConfigError(`--framework ${JSON.stringify(v)}: expected vue or svelte`);
  return framework;
}

/** The template in `framework`: minimal comes in both, the others in one. Without a template, the first offered in it. */
export function templateFor(template: Template | undefined, framework: Framework): Template {
  if (template === undefined) return OFFERED.find((t) => TEMPLATE_INFO[t].framework === framework) ?? "showcase";
  if (template === "minimal" || template === "svelte") return framework === "svelte" ? "svelte" : "minimal";
  if (TEMPLATE_INFO[template].framework === framework) return template;
  throw new ConfigError(
    `--framework ${framework}: the ${template} template is a ${TEMPLATE_INFO[template].framework === "vue" ? "Vue" : "Svelte"} app. minimal comes in both`,
  );
}

function answered<T>(value: p.Answer<T>): T {
  if (value === p.CANCEL) {
    p.cancel("Nothing was created.");
    process.exit(130);
  }
  return value;
}

/** `sk create`: the questions the flags leave open, then {@link create}. */
export async function createCommand(target: string | undefined, flags: CreateFlags): Promise<number> {
  const ask = !flags.yes && process.stdin.isTTY === true && process.stdout.isTTY === true;
  const framework = flags.framework !== undefined ? parseFramework(flags.framework) : undefined;
  const template = flags.template !== undefined ? parseTemplate(flags.template) : undefined;
  const options: CreateOptions = {
    name: flags.name,
    template: framework ? templateFor(template, framework) : template,
    platforms: flags.os
      ?.split(/[\s,]+/)
      .filter(Boolean)
      .map(parsePlatform),
    keepDevtools: flags.devtools,
    packageManager: flags.pm !== undefined ? parsePm(flags.pm) : undefined,
  };
  let dir = target;

  if (ask) {
    p.intro("sk create");
    dir ??= answered(
      await p.text({
        message: "Where should the app go?",
        placeholder: "my-app",
        fallback: "my-app",
        validate: (v) => (existsSync(join(resolve(v), "package.json")) ? `${v} already has a package.json` : undefined),
      }),
    );
    options.name ??= answered(
      await p.text({
        message: "App name",
        initial: basename(resolve(dir)),
        validate: (v) => (v ? undefined : "it needs a name"),
      }),
    );
    options.template ??= answered(
      await p.select<Template>({
        message: "Template",
        options: OFFERED.map((t) => ({ value: t, label: t, hint: TEMPLATE_INFO[t].hint })),
        initial: "showcase",
      }),
    );
    if (options.template === "minimal" && framework === undefined) {
      const picked = answered(
        await p.select<Framework>({
          message: "Framework",
          options: [
            { value: "vue", label: "Vue" },
            { value: "svelte", label: "Svelte 5" },
          ],
          initial: "vue",
        }),
      );
      options.template = templateFor("minimal", picked);
    }
    options.platforms ??= answered(
      await p.multiselect<Platform>({
        message: "Package it for",
        options: [
          { value: "windows", label: "Windows" },
          { value: "macos", label: "macOS" },
          { value: "linux", label: "Linux" },
        ],
        initial: [currentPlatform()],
        required: true,
      }),
    );
    options.keepDevtools ??= answered(
      await p.confirm({
        message: "Keep devtools (F12) in published builds? sk build always has them",
        initial: false,
      }),
    );
    options.packageManager ??= answered(
      await p.select<PackageManager>({
        message: "Package manager",
        options: PACKAGE_MANAGERS.map((pm) => ({ value: pm, label: pm })),
        initial: detectPackageManager(),
      }),
    );
  }

  dir ??= "my-app";
  const pm = options.packageManager ?? detectPackageManager();
  await prepareTemplate(options.template ?? "showcase");
  create(dir, options);
  const where = relative(process.cwd(), resolve(dir)) || ".";
  const next = [...(where === "." ? [] : [`cd ${where}`]), `${pm} install`, runScript(pm, "dev")].join("\n");
  if (ask) {
    p.note(next, "Next");
    p.outro(`${options.name ?? basename(resolve(dir))} is ready`);
  } else {
    const c = paint(colorEnabled(process.stdout, flags.color));
    console.log(`${c.green("✓")} created ${c.cyan(resolve(dir))}\n  ${c.dim("next:")} ${next.replace(/\n/g, " && ")}`);
  }
  return 0;
}
