#!/usr/bin/env bun
// sk — create, develop and package a Skathi app for Windows, macOS or Linux, without a
// Rust toolchain. Every command and option is declared once here; help.ts lays the help out.

import { spawn } from "node:child_process";
import { Command, CommanderError, Option, type CommandUnknownOpts } from "@commander-js/extra-typings";
import { aotFlags } from "./aot-build";
import { buildApp, summary } from "./build-app";
import {
  ConfigError,
  noJitVariant,
  parseVariant,
  PARTS_NOT_SELECTABLE,
  platformVariants,
  variantList,
  VARIANTS,
  type Variant,
} from "./config";
import { createCommand } from "./create";
import { dev } from "./dev";
import { helpFormat, section } from "./help";
import { addHost, BUN_RANGE, buildHost, CLI_VERSION, DEV_HOST, HostError, hostInfo, resolveHost } from "./host";
import { fetchHost } from "./host-fetch";
import { currentPlatform, parsePlatform } from "./platform";
import { colorEnabled, lineProgress, logTo, paint, progressFor } from "./term";
import { PIN } from "../effects/redslangc";
import { cacheRoots, formatInventory, formatPrune, inventory, needed, prune } from "./workload-cache";

interface Run {
  /** What follows `--`: the app's own arguments for `sk run`. */
  rest: string[];
  color: boolean | undefined;
  code: number;
  /** The command being parsed, for usage errors. */
  at: string;
}

const track = (run: Run) => (_: CommandUnknownOpts, sub: CommandUnknownOpts) => {
  run.at = `${run.at} ${sub.name()}`;
};

/** `--color` / `--no-color` anywhere on the line, read before parsing so usage errors are coloured too. */
export const colorArg = (args: string[]): boolean | undefined =>
  args.includes("--no-color") ? false : args.includes("--color") ? true : undefined;

const INSTALLABLE: readonly Variant[] = [...platformVariants(currentPlatform()), DEV_HOST];
const BY_DEFAULT = (variants: readonly Variant[]) => variants.filter((v) => !noJitVariant(v));

function installable(v: string): Variant {
  const name = v.trim();
  if (INSTALLABLE.includes(name)) return name;
  throw new ConfigError(`${JSON.stringify(v)} is not a workload variant — ${INSTALLABLE.join(", ")}`);
}

function refuseParts(o: { without?: unknown; features?: unknown }): void {
  if (o.without !== undefined) throw new ConfigError(`--without: ${PARTS_NOT_SELECTABLE}`);
  if (o.features !== undefined) throw new ConfigError(`--features: ${PARTS_NOT_SELECTABLE}`);
}

const hidden = <Flags extends string>(flags: Flags) => new Option(flags).hideHelp();

const collect = (value: string, all: string[]) => [...all, value];

/** The options of `names` given on the line, for pairs that contradict each other. */
function given(cmd: CommandUnknownOpts, ...names: string[]): Set<string> {
  const seen = new Set<string>();
  for (const name of names) cmd.on(`option:${name}`, () => seen.add(name));
  return seen;
}

/** Usage errors of `sk check` and `sk explain` exit 2, as their reports do. */
const exitWith =
  (code: number) =>
  (e: CommanderError): never => {
    throw e.exitCode === 0 ? e : new CommanderError(code, e.code, e.message);
  };

const BUILD_MODES = {
  build: {
    summary: "Debug build: devtools on, not minified",
    about: "A debug build in <app>/dist/debug: devtools (F12) on, the bundle not minified.",
  },
  publish: {
    summary: "Release build to ship",
    about:
      'The release build in <app>/dist/release: minified, devtools only with "publishDevtools": true in sk.config.json.',
  },
  run: {
    summary: "Build, then start the app",
    about: "sk build, then start the app with its output in this terminal. Arguments after -- go to the app.",
  },
} as const;

const CHECK_EXIT_CODES: [string, string][] = [
  ["0", "Clean"],
  ["1", "Errors (or warnings with --deny-warnings)"],
  ["2", "Bad usage or config, or the check failed"],
];

const RENAMED: Record<string, string> = { fetch: "install", path: "path", info: "info", add: "add", build: "build" };

function pruneCommand(parent: CommandUnknownOpts, run: Run) {
  return new Command("prune")
    .copyInheritedSettings(parent)
    .summary("Remove cached workloads this sk does not use")
    .description(
      `Remove what sk ${CLI_VERSION} does not need: other versions' workloads and sk-aot, redslangc builds other than ${PIN.version}, incomplete downloads, ` +
        "native images expanded by earlier versions or by an inspector that carries a compressed one (expanded again when needed) and old toolchains.",
    )
    .option("--dry-run", "Show what would go, remove nothing")
    .option("--keep <version>", "Keep this version's workloads too; repeatable", collect, [])
    .option("--all", "Remove this version's workloads as well")
    .action((o) => {
      const roots = cacheRoots();
      const options = { current: CLI_VERSION, keep: o.keep, all: o.all === true, dryRun: o.dryRun === true, roots };
      const result = prune(inventory(roots), options);
      console.log(formatPrune(result, options, colorEnabled(process.stdout, run.color)));
    });
}

export function cli(run: Run): Command {
  const outColor = colorEnabled(process.stdout, run.color);
  const errColor = colorEnabled(process.stderr, run.color);
  const p = paint(outColor);
  const program = new Command("sk")
    .description("Skathi apps for Windows, macOS and Linux")
    .version(CLI_VERSION, "-v, --version", "Print the version")
    .addHelpOption(hidden("-h, --help"))
    .option("--no-color", "Plain output (--color forces colour)")
    .addOption(hidden("--color"))
    .configureHelp(helpFormat(outColor || errColor))
    .configureOutput({
      writeOut: (s) => process.stdout.write(s),
      writeErr: (s) => process.stderr.write(s),
      outputError: () => {},
      getOutHasColors: () => outColor,
      getErrHasColors: () => errColor,
    })
    .showSuggestionAfterError()
    .exitOverride()
    .hook("preSubcommand", track(run))
    .addHelpText(
      "after",
      () => `\n${p.dim("Run sk <command> --help for its options. [app] is a directory (default: .).")}`,
    );

  program
    .command("create")
    .summary("Start a new app")
    .description("Start a new app. In a terminal it asks for whatever the options leave open.")
    .argument("[dir]")
    .option("--name <name>", "The app's name (default: the directory's)")
    .option(
      "--template <name>",
      "showcase (every feature), minimal (Vue or Svelte), player (Svelte music player), weather (Vue weather app), draw (Vue pen canvas on WebGPU), chip8 (Vue CHIP-8 emulator), sequencer (Svelte drum machine) or rv32 (Vue RISC-V emulator running the Doom engine)",
    )
    .option("--framework <name>", "vue or svelte, for minimal (default: vue)")
    .option("--os <list>", "What to package for: windows, macos, linux")
    .option("--devtools", "Keep devtools (F12) in published builds")
    .addOption(hidden("--no-devtools"))
    .option("--pm <name>", "bun, npm, pnpm or yarn")
    .option("-y, --yes", "Ask nothing, take the defaults")
    .addOption(hidden("--without [parts]"))
    .addOption(hidden("--features [parts]"))
    .action(async (dir, o) => {
      refuseParts(o);
      run.code = await createCommand(dir, {
        name: o.name,
        template: o.template,
        framework: o.framework,
        os: o.os,
        devtools: o.devtools,
        pm: o.pm,
        yes: o.yes === true,
        color: run.color,
      });
    });

  program
    .command("dev")
    .summary("Run the app with hot reload")
    .description(
      "Run the app with hot reload: edits to .vue, .ts and .css apply without a restart; an edit to a shaders/*.slang effect recompiles it and reloads it in the running app.",
    )
    .argument("[app]")
    .option("--exe <workload>", "Run this dev workload instead of the cached one")
    .option("--shader-compiler <exe>", "Compile effect shaders with this redslangc (or SK_REDSLANGC)")
    .action(async (app, o) => {
      run.code = await dev(app ?? ".", { exe: o.exe, shaderCompiler: o.shaderCompiler, color: run.color });
    });

  program
    .command("types")
    .summary("Types for the native modules of an embedding program")
    .description(
      "Writes src/native-modules.d.ts: the `native:*` modules a program built with the skathi crate gives the app, as `<program> --sk-host-types` declares them. `sk dev --exe <program>` writes it too.",
    )
    .argument("[app]")
    .requiredOption("--exe <program>", "The embedding program")
    .action(async (app, o) => {
      const { typesCommand } = await import("./types");
      run.code = typesCommand(app ?? ".", o.exe);
    });

  for (const [mode, text] of Object.entries(BUILD_MODES)) {
    const cmd = program
      .command(mode)
      .summary(text.summary)
      .description(text.about)
      .argument("[app]")
      .option("--platform <os>", "windows, macos or linux (default: this OS)")
      .option("--out <dir>", "Output directory (default: <app>/dist/debug or dist/release)")
      .option("--define <K=V>", "A Vite-style define; repeatable", collect, [])
      .option("--config <path>", "An sk.config.json outside the app")
      .option("--sign <identity>", "macOS: codesign identity (- for ad hoc)")
      .option("--no-sign", "macOS: leave the bundle unsigned")
      .option("--console", "Windows: keep the console window")
      .option("--aot", "Compile the app to a native image on Windows x64 or macOS arm64")
      .addOption(hidden("--no-aot"))
      .option("--json", "Print the result as JSON")
      .optionsGroup("Advanced")
      .option("--workload <exe>", "Stamp this workload, or use it to target an embedder with --aot --payload-only")
      .option("--payload-only", "Write app.skpk and any sidecars to dist/<mode>/.sk without stamping an executable")
      .option("--aot-inputs", "Keep the typed sources for a later AOT compile")
      .option("--aot-compiler <exe>", "Compile native images with this sk-aot binary (or SK_AOT_COMPILER)")
      .option("--shader-compiler <exe>", "Compile effect shaders with this redslangc (or SK_REDSLANGC)")
      .addOption(hidden("--host <exe>"))
      .addOption(hidden("--variant <name>"))
      .addOption(hidden("--allow-workload-mismatch"))
      .addOption(hidden("--allow-host-mismatch"))
      .addOption(hidden("--without [parts]"))
      .addOption(hidden("--features [parts]"));
    const seen = given(cmd, "aot", "no-aot", "sign", "no-sign");
    cmd.action(async (app, o) => {
      refuseParts(o);
      const platform = o.platform === undefined ? undefined : parsePlatform(o.platform);
      if (mode === "run" && platform && platform !== currentPlatform()) {
        throw new ConfigError(
          `sk run starts the app here; a ${platform} build cannot run on ${process.platform} — use sk build`,
        );
      }
      if (seen.has("sign") && seen.has("no-sign")) throw new ConfigError("--sign and --no-sign contradict each other");
      const sign = typeof o.sign === "string" ? o.sign.trim() : o.sign === false ? false : undefined;
      if (sign === "") throw new ConfigError('--sign needs a codesign identity ("-" for ad hoc)');
      const aot = aotFlags({
        aot: seen.has("aot"),
        "no-aot": seen.has("no-aot"),
        ...(o.aotCompiler !== undefined && { "aot-compiler": o.aotCompiler }),
      });
      const progress = progressFor(process.stderr, errColor);
      const result = await buildApp({
        progress,
        target: app ?? ".",
        configPath: o.config,
        out: o.out,
        host: o.workload ?? o.host,
        variant: o.variant === undefined ? undefined : parseVariant(o.variant, "--variant"),
        console: o.console === true,
        payloadOnly: o.payloadOnly === true,
        allowHostMismatch: o.allowWorkloadMismatch === true || o.allowHostMismatch === true,
        defines: o.define,
        aotInputs: o.aotInputs === true,
        shaderCompiler: o.shaderCompiler,
        ...aot,
        color: errColor,
        platform,
        sign,
        mode: mode === "publish" ? "release" : "debug",
      }).finally(() => progress.end());
      if (o.json) {
        const s = result.stamp;
        console.log(
          JSON.stringify({
            exe: result.exe,
            skpk: result.skpk,
            aotInputs: result.aotInputs,
            effects: result.effects,
            native: result.native,
            name: result.config.name,
            productName: result.config.productName,
            version: result.config.version,
            variant: result.variant,
            host: result.host && {
              exe: result.host.exe,
              source: result.host.source,
              profile: result.host.info.profile,
            },
            assets: result.assets.length,
            bundleBytes: result.bundleBytes,
            payloadBytes: result.payloadBytes,
            ...(result.packs.length ? { packs: result.packs } : {}),
            ...(result.icu ? { icu: result.icu } : {}),
            stamp: s && {
              bytes: s.bytes,
              skew: s.skew,
              dataAligned: s.dataAligned,
              passes: s.passes,
              subsystem: s.subsystem,
              iconCount: s.iconCount,
              ms: s.ms,
            },
            ...(result.bundle && {
              platform: result.platform,
              app: result.bundle.app,
              bundle: {
                bundleId: result.bundle.bundleId,
                bytes: result.bundle.bytes,
                iconSizes: result.bundle.iconSizes,
                signed: result.bundle.signed,
                signNote: result.bundle.signNote,
                entitlements: result.bundle.entitlements,
                ms: result.bundle.ms,
              },
            }),
            warnings: result.warnings,
            ms: result.ms,
          }),
        );
      } else {
        console.log(summary(result, outColor));
      }
      if (mode !== "run" || !result.exe) return;
      // Inherited handles, so a GUI-subsystem exe's stderr still lands in this terminal.
      const child = spawn(result.exe, run.rest, { stdio: "inherit" });
      run.code = await new Promise<number>((done) => child.on("exit", (c) => done(c ?? 0)));
    });
  }

  const colorFlags = { color: run.color === true, "no-color": run.color === false };
  program
    .command("check")
    .summary("What breaks under AOT, and how well the code is typed")
    .description(
      "What in the bundle, app code and node_modules, breaks or slows down under AOT, and how well it is typed.",
    )
    .argument("[app]")
    .option("--format <format>", "json: one JSON report on stdout (schema skathi.check)")
    .option("--deny-warnings", "Fail on warnings too")
    .option("--no-deps", "Only the app's own code")
    .option("--config <path>", "An sk.config.json outside the app")
    .addHelpText("after", () => section(p, "Exit codes", CHECK_EXIT_CODES).join("\n"))
    .exitOverride(exitWith(2))
    .action(async (app, o) => {
      const { checkCommand } = await import("../check/command");
      run.code = await checkCommand(
        app,
        { format: o.format, config: o.config, "deny-warnings": o.denyWarnings, "no-deps": !o.deps, ...colorFlags },
        CLI_VERSION,
      );
    });

  program
    .command("explain")
    .summary("What a check code means and how to fix it")
    .description("What a check code means, an example, and how to fix it.")
    .argument("[code]")
    .option("--list", "Every code")
    .exitOverride(exitWith(2))
    .action(async (code, o) => {
      run.code = (await import("../check/command")).explainCommand(code, o.list === true, colorFlags);
    });

  const workload = program
    .command("workload")
    .summary("Install, list and prune the engine workloads")
    .description(
      "A workload is the prebuilt engine executable an app is packed into. sk build and sk dev install " +
        "this version's on demand.",
    )
    .hook("preSubcommand", track(run));

  workload
    .command("list")
    .description("What is cached, its size, and what sk prune would remove")
    .option("--json", "Print the inventory as JSON")
    .action((o) => {
      const roots = cacheRoots();
      const entries = inventory(roots);
      const keep = { current: CLI_VERSION };
      if (o.json) {
        const listed = entries.map((e) => ({ ...e, needed: needed(e, keep) }));
        console.log(JSON.stringify({ root: roots.workloads, version: CLI_VERSION, entries: listed }, null, 2));
      } else console.log(formatInventory(entries, roots, keep, outColor));
    });

  workload
    .command("install")
    .description(`Download this version's workloads (default: ${BY_DEFAULT(INSTALLABLE).join(", ")})`)
    .argument("[variants...]")
    .action(async (variants) => {
      const progress = progressFor(process.stderr, errColor);
      try {
        for (const variant of variants.length ? variants.map(installable) : BY_DEFAULT(INSTALLABLE)) {
          progress.step(`${variant} workload`);
          const { path } = await fetchHost({ variant, onBytes: (done, total) => progress.bytes(done, total) });
          progress.detail(path);
        }
        progress.end();
      } catch (e) {
        progress.fail();
        throw e;
      }
    });

  workload
    .command("add")
    .description("Verify a workload and add it to the cache")
    .argument("<exe>")
    .action((exe) => {
      addHost(exe, (line) => console.log(line));
    });

  workload
    .command("build")
    .description(`In a Skathi checkout: build workloads with cargo (default: ${BY_DEFAULT(VARIANTS).join(" and ")})`)
    .argument("[variants...]")
    .action((variants) => {
      // cargo draws its own progress on the terminal: plain lines around it.
      const lines = lineProgress((line) => console.error(line), errColor);
      try {
        const chosen = variants.length
          ? variants.map((v) => parseVariant(v, "sk workload build"))
          : BY_DEFAULT(VARIANTS);
        for (const variant of chosen) {
          lines.step(`${variant} workload`);
          lines.detail(buildHost(variant, logTo(lines)).path);
        }
        lines.end();
      } catch (e) {
        lines.fail();
        throw e;
      }
    });

  workload
    .command("path")
    .description("Which workload sk build would use")
    .option("--variant <name>", `${variantList(VARIANTS)} (default: standard)`)
    .option("--workload <exe>", "Check this one instead of looking for it")
    .option("--platform <os>", "windows, macos or linux (default: this OS)")
    .option("--json", "Print it with what it says about itself")
    .addOption(hidden("--host <exe>"))
    .action((o) => {
      const h = resolveHost({
        variant: o.variant === undefined ? "standard" : parseVariant(o.variant, "--variant"),
        host: o.workload ?? o.host,
        platform: o.platform === undefined ? undefined : parsePlatform(o.platform),
      });
      console.log(o.json ? JSON.stringify({ exe: h.exe, source: h.source, info: h.info }, null, 2) : h.exe);
    });

  workload
    .command("info")
    .description("What a workload says about itself (default: the one sk build would use)")
    .argument("[exe]")
    .option("--variant <name>", `${variantList(VARIANTS)} (default: standard)`)
    .action((exe, o) => {
      const variant = o.variant === undefined ? "standard" : parseVariant(o.variant, "--variant");
      // `hostInfo` refuses a stamped exe itself: asking one for its info would launch the app inside it.
      console.log(JSON.stringify(hostInfo(exe ?? resolveHost({ variant, allowMismatch: true }).exe), null, 2));
    });

  workload.addCommand(pruneCommand(workload, run));
  program.addCommand(pruneCommand(program, run));

  program
    .command("host", { hidden: true })
    .argument("[command]")
    .argument("[args...]")
    .helpOption(false)
    .allowUnknownOption()
    .action((sub) => {
      throw new HostError(`sk host is now sk workload: sk workload ${RENAMED[sub ?? ""] ?? "--help"}`);
    });

  // Listed for --help only: main() hands `sk ctl` to the devtools client before commander parses.
  program
    .command("ctl")
    .summary("Drive a running app: read its tree, click, evaluate, take screenshots")
    .argument("[command]")
    .argument("[args...]")
    .helpOption(false)
    .allowUnknownOption();

  program.command("version", { hidden: true }).action(() => console.log(CLI_VERSION));

  return program;
}

function usageError(e: CommanderError, at: string, color: boolean): void {
  const [first = "", ...more] = e.message.replace(/^error: /, "").split("\n");
  const unknown = /^unknown command '(.*)'$/.exec(first);
  const text = unknown
    ? `unknown command ${JSON.stringify(unknown[1])} (${at} --help lists them)`
    : `${first} (${at} --help lists the options)`;
  console.error([`${paint(color).red("error")}: ${text}`, ...more].join("\n"));
}

export async function main(argv: string[]): Promise<number> {
  if (argv[0] === "ctl") {
    const client = Bun.fileURLToPath(new URL("../devtools/ctl.ts", import.meta.url));
    const done = Bun.spawnSync([process.execPath, client, ...argv.slice(1)], {
      stdio: ["inherit", "inherit", "inherit"],
    });
    return done.exitCode;
  }
  const split = argv.indexOf("--");
  const head = split < 0 ? argv : argv.slice(0, split);
  const run: Run = { rest: split < 0 ? [] : argv.slice(split + 1), color: colorArg(head), code: 0, at: "sk" };
  try {
    await cli(run).parseAsync(head, { from: "user" });
  } catch (e) {
    if (!(e instanceof CommanderError)) throw e;
    const shown = e.code === "commander.helpDisplayed" || e.code === "commander.help" || e.code === "commander.version";
    if (!shown) usageError(e, run.at, colorEnabled(process.stderr, run.color));
    return e.exitCode;
  }
  return run.code;
}

if (import.meta.main) {
  if (!Bun.semver.satisfies(Bun.version, BUN_RANGE)) {
    // Package scripts run the first `bun` executable on PATH, which can be an older install.
    console.error(
      `error: Skathi needs Bun ${BUN_RANGE}, but this is Bun ${Bun.version} (${process.execPath}). ` +
        `Upgrade it (\`"${process.execPath}" upgrade\`) or put a newer Bun first on PATH.`,
    );
    process.exit(1);
  }
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e) => {
      const known = e instanceof ConfigError || e instanceof HostError;
      const red = paint(colorEnabled(process.stderr, colorArg(process.argv.slice(2)))).red;
      if (known || (e instanceof Error && !process.env.SK_DEBUG)) console.error(`${red("error")}: ${e.message}`);
      else console.error(e);
      process.exit(1);
    },
  );
}
