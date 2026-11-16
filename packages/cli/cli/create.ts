// `sk create [dir]`: a new app. In a terminal it asks for what the flags do not say: the directory, the
// name, the template, the OSes to package for, whether published builds keep devtools,
// and the package manager. `--yes` (or no terminal) takes the defaults.
//
// It copies templates/<template>/ (src/, assets/) with the placeholders filled in, and writes
// package.json (the app root the builders resolve npm dependencies from), sk.config.json, icon.png (the
// template's own, else the Skathi icon) and a tsconfig.json that type-checks against the runtime.
// Nothing is overwritten: an existing file is an error, not a merge.

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, extname, join, relative, resolve } from "node:path";
import * as p from "@clack/prompts";
import { runtimeRequire } from "../runtime-path";
import { ConfigError, sanitizeExeName } from "./config";
import { CLI_VERSION } from "./host";
import { currentPlatform, parsePlatform, type Platform } from "./platform";
import { colorEnabled, paint } from "./term";

export const PACKAGE_MANAGERS = ["bun", "npm", "pnpm", "yarn"] as const;
export type PackageManager = (typeof PACKAGE_MANAGERS)[number];

export const TEMPLATES = ["showcase", "minimal", "svelte", "player"] as const;
export type Template = (typeof TEMPLATES)[number];

interface Checker {
  script: string;
  devDependencies: Record<string, string>;
  /** The component files the app's tsconfig.json includes besides its .ts sources. */
  sources: string;
}

const VUE_TSC: Checker = { script: "vue-tsc --noEmit", devDependencies: { "vue-tsc": "^3" }, sources: "src/**/*.vue" };
const SVELTE_CHECK: Checker = {
  script: "svelte-check",
  devDependencies: { "svelte-check": "^4" },
  sources: "src/**/*.svelte",
};

interface TemplateInfo {
  hint: string;
  /** Taken at the versions the runtime itself depends on, or accepts as a peer. */
  dependencies: string[];
  window: Record<string, unknown>;
  packs?: { name: string; match?: string[] }[];
  checker: Checker;
}

const TEMPLATE_INFO: Record<Template, TemplateInfo> = {
  showcase: {
    hint: "every Skathi feature, one page each",
    dependencies: ["pinia", "vue-router"],
    window: { width: 1100, height: 720, minWidth: 480, minHeight: 360, titleBarStyle: "hidden", titleBarHeight: 40 },
    packs: [{ name: "media", match: ["assets/media/**"] }],
    checker: VUE_TSC,
  },
  minimal: {
    hint: "one window, one component",
    dependencies: [],
    window: { width: 760, height: 520, titleBarStyle: "default" },
    checker: VUE_TSC,
  },
  svelte: {
    hint: "the minimal app in Svelte 5",
    dependencies: ["svelte"],
    window: { width: 760, height: 520, titleBarStyle: "default" },
    checker: SVELTE_CHECK,
  },
  player: {
    hint: "a music player in Svelte 5, dressed in shaders",
    dependencies: ["svelte"],
    window: { width: 1100, height: 760, minWidth: 480, minHeight: 640, titleBarStyle: "hidden", titleBarHeight: 44 },
    packs: [{ name: "media", match: ["assets/music/**", "assets/covers/**"] }],
    checker: SVELTE_CHECK,
  },
};

export const TEMPLATES_DIR = resolve(import.meta.dir, "..", "templates");

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
  const dir = join(TEMPLATES_DIR, template);
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
  const runtimePkg = runtimeRequire("./package.json") as {
    dependencies: Record<string, string>;
    peerDependencies?: Record<string, string>;
  };
  const runtimeDeps = { ...runtimePkg.peerDependencies, ...runtimePkg.dependencies };
  const sdk = `^${CLI_VERSION}`;

  const files: Record<string, string | Uint8Array> = {
    "icon.png": readFileSync(join(TEMPLATES_DIR, "icon.png")),
    ...templateFiles(template, { name, productName }),
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
      },
      dependencies: {
        "@skathi-native/runtime": sdk,
        ...Object.fromEntries(info.dependencies.map((dep) => [dep, runtimeDeps[dep]])),
      },
      devDependencies: { "@skathi-native/cli": sdk, typescript: "^5", ...info.checker.devDependencies },
    }),
    "sk.config.json": json({
      $schema: CONFIG_SCHEMA,
      name,
      productName,
      version: "0.1.0",
      platforms,
      publishDevtools: options.keepDevtools === true,
      window: { title: productName, ...info.window, backdrop: "mica" },
      assets: ["assets"],
      ...(info.packs ? { packs: info.packs } : {}),
      windows: { exeName, icon: "icon.png" },
      ...(platforms.includes("macos") ? { macos: { bundleId: `com.example.${slug}` } } : {}),
    }),
    "tsconfig.json": json({
      extends: "@skathi-native/runtime/tsconfig.app.json",
      include: ["src/**/*.ts", info.checker.sources],
    }),
    ".gitignore": "node_modules/\ndist/\n",
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
    template: flags.template !== undefined ? parseTemplate(flags.template) : undefined,
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
    options.template ??= answered(
      await p.select<Template>({
        message: "Template",
        options: TEMPLATES.map((t) => ({ value: t, label: t, hint: TEMPLATE_INFO[t].hint })),
        initialValue: "showcase",
      }),
    );
    options.platforms ??= answered(
      await p.multiselect<Platform>({
        message: "Package it for",
        options: [
          { value: "windows", label: "Windows" },
          { value: "macos", label: "macOS" },
          { value: "linux", label: "Linux" },
        ],
        initialValues: [currentPlatform()],
        required: true,
      }),
    );
    options.keepDevtools ??= answered(
      await p.confirm({
        message: "Keep devtools (F12) in published builds? sk build always has them",
        initialValue: false,
      }),
    );
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
