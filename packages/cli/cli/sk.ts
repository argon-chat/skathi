#!/usr/bin/env bun
// sk — create, develop and package a Skathi app for Windows, macOS or Linux, without a
// Rust toolchain.

import { spawn } from "node:child_process";
import { buildApp, summary } from "./build-app";
import {
  ConfigError,
  OPTIONAL,
  parseParts,
  partsLeftOut,
  selectVariant,
  VARIANTS,
  variantFor,
  type Variant,
} from "./config";
import { dev } from "./dev";
import { addHost, buildHost, CLI_VERSION, DEV_HOST, HostError, hostInfo, resolveHost } from "./host";
import { fetchHost } from "./host-fetch";
import { colorEnabled, colorFlag, lineProgress, logTo, paint, progressFor } from "./term";
import { createCommand } from "./create";
import { currentPlatform, parsePlatform } from "./platform";
import { commandHelp, topHelp } from "./help";

type Flags = Record<string, string | boolean>;
const BOOLEAN = new Set([
  "deny-warnings",
  "no-deps",
  "list",
  "console",
  "payload-only",
  "allow-host-mismatch",
  "aot-inputs",
  "help",
  "version",
  "json",
  "no-sign",
  "yes",
  "devtools",
  "no-devtools",
  "color",
  "no-color",
]);
const REPEATABLE = new Set(["define"]);

function parse(argv: string[]): {
  positional: string[];
  flags: Flags;
  repeated: Record<string, string[]>;
  rest: string[];
} {
  const positional: string[] = [];
  const flags: Flags = {};
  const repeated: Record<string, string[]> = {};
  let rest: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]!;
    if (a === "--") {
      rest = argv.slice(i + 1);
      break;
    }
    if (a === "-h") flags.help = true;
    else if (a === "-y") flags.yes = true;
    else if (a === "-v") flags.version = true;
    else if (a.startsWith("--")) {
      // Split at the FIRST "=" only: `--define=K=V` keeps its own "=".
      const body = a.slice(2);
      const eq = body.indexOf("=");
      const name = eq < 0 ? body : body.slice(0, eq);
      const inline = eq < 0 ? undefined : body.slice(eq + 1);
      if (BOOLEAN.has(name)) flags[name] = true;
      else if (REPEATABLE.has(name)) (repeated[name] ??= []).push(inline ?? argv[++i] ?? "");
      else flags[name] = inline ?? argv[++i] ?? "";
    } else positional.push(a);
  }
  return { positional, flags, repeated, rest };
}

function variantOf(flags: Flags): Variant | undefined {
  const list = (name: string) => {
    const v = flags[name];
    if (v === undefined) return undefined;
    if (typeof v !== "string" || !v.trim())
      throw new ConfigError(`--${name} needs a list of parts (${OPTIONAL.join(", ")})`);
    return parseParts(v, `--${name}`);
  };
  const host = flags.variant;
  if (host !== undefined && (typeof host !== "string" || !host.trim()))
    throw new ConfigError("--variant needs a variant name");
  const sel = { features: list("features"), without: list("without"), host: host as string | undefined };
  if (!sel.features && !sel.without && !sel.host) return undefined;
  // Normalised: `no-audio+no-devtools` and `no-devtools+no-audio` are one host.
  return variantFor(partsLeftOut(selectVariant(sel, "the command line")));
}

async function main(argv: string[]): Promise<number> {
  const { positional, flags, repeated, rest } = parse(argv);
  const [command, ...args] = positional;
  if (command === "version" || (!command && flags.version === true)) {
    console.log(CLI_VERSION);
    return 0;
  }
  const color = colorEnabled(process.stdout, colorFlag(flags));
  const about = command === "help" ? args[0] : flags.help ? command : undefined;
  if (about !== undefined) {
    const text = commandHelp(about, color);
    if (text) {
      console.log(text);
      return 0;
    }
  }
  if (!command || flags.help || command === "help") {
    console.log(topHelp(color));
    return command || flags.help ? 0 : 1;
  }

  if (command === "build" || command === "publish" || command === "run") {
    const platform = flags.platform === undefined ? undefined : parsePlatform(flags.platform);
    if (command === "run" && platform && platform !== currentPlatform()) {
      throw new ConfigError(
        `sk run starts the app here; a ${platform} build cannot run on ${process.platform} — use sk build`,
      );
    }
    if (flags.sign !== undefined && (typeof flags.sign !== "string" || !flags.sign.trim())) {
      throw new ConfigError('--sign needs a codesign identity ("-" for ad hoc)');
    }
    if (flags.sign !== undefined && flags["no-sign"])
      throw new ConfigError("--sign and --no-sign contradict each other");
    const progress = progressFor(process.stderr, colorEnabled(process.stderr, colorFlag(flags)));
    const result = await buildApp({
      progress,
      target: args[0] ?? ".",
      configPath: typeof flags.config === "string" ? flags.config : undefined,
      out: typeof flags.out === "string" ? flags.out : undefined,
      host: typeof flags.host === "string" ? flags.host : undefined,
      variant: variantOf(flags),
      console: flags.console === true,
      payloadOnly: flags["payload-only"] === true,
      allowHostMismatch: flags["allow-host-mismatch"] === true,
      defines: repeated.define,
      aotInputs: flags["aot-inputs"] === true,
      platform,
      sign: flags["no-sign"] ? false : typeof flags.sign === "string" ? flags.sign.trim() : undefined,
      mode: command === "publish" ? "release" : "debug",
    }).finally(() => progress.end());
    if (flags.json) {
      const s = result.stamp;
      console.log(
        JSON.stringify({
          exe: result.exe,
          skpk: result.skpk,
          aotInputs: result.aotInputs,
          name: result.config.name,
          productName: result.config.productName,
          version: result.config.version,
          variant: result.variant,
          host: result.host && { exe: result.host.exe, source: result.host.source, profile: result.host.info.profile },
          assets: result.assets.length,
          bundleBytes: result.bundleBytes,
          payloadBytes: result.payloadBytes,
          ...(result.packs.length ? { packs: result.packs } : {}),
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
      console.log(summary(result, colorEnabled(process.stdout, colorFlag(flags))));
    }
    if (command !== "run" || !result.exe) return 0;
    // `run`: inherited handles, so a GUI-subsystem exe's stderr still lands in this terminal.
    const child = spawn(result.exe, rest, { stdio: "inherit" });
    return await new Promise<number>((done) => child.on("exit", (c) => done(c ?? 0)));
  }

  if (command === "check") return await (await import("../check/command")).checkCommand(args[0], flags, CLI_VERSION);
  if (command === "explain")
    return (await import("../check/command")).explainCommand(args[0], flags.list === true, flags);

  if (command === "dev") {
    return await dev(args[0] ?? ".", { exe: typeof flags.exe === "string" ? flags.exe : undefined });
  }

  if (command === "host") {
    const [sub, arg] = args;
    if (sub === "info") {
      const exe = arg ?? resolveHost({ variant: variantOf(flags) ?? "standard", allowMismatch: true }).exe;
      // `hostInfo` refuses a stamped exe itself: asking one for host info would launch the app inside it.
      console.log(JSON.stringify(hostInfo(exe), null, 2));
      return 0;
    }
    if (sub === "path") {
      const h = resolveHost({
        variant: variantOf(flags) ?? "standard",
        host: typeof flags.host === "string" ? flags.host : undefined,
        platform: flags.platform === undefined ? undefined : parsePlatform(flags.platform),
      });
      console.log(flags.json ? JSON.stringify({ exe: h.exe, source: h.source, info: h.info }, null, 2) : h.exe);
      return 0;
    }
    if (sub === "add") {
      if (!arg) throw new HostError("sk host add <exe>");
      addHost(arg, (line) => console.log(line));
      return 0;
    }
    const color = colorEnabled(process.stderr, colorFlag(flags));
    if (sub === "fetch") {
      const named = flags.variant === "both" || flags.variant === DEV_HOST ? flags.variant : variantOf(flags);
      const variants = named === undefined ? [...VARIANTS, DEV_HOST] : named === "both" ? VARIANTS : [named];
      const progress = progressFor(process.stderr, color);
      try {
        for (const variant of variants) {
          progress.step(`${variant} host`);
          await fetchHost({ variant, log: (line) => progress.detail(line) });
        }
      } finally {
        progress.end();
      }
      return 0;
    }
    if (sub === "build") {
      // cargo draws its own progress on the terminal: plain lines around it.
      const lines = lineProgress((line) => console.error(line), color);
      const v = flags.variant === "both" ? "both" : (variantOf(flags) ?? "both");
      for (const variant of v === "both" ? VARIANTS : [v]) {
        lines.step(`${variant} host`);
        const built = buildHost(variant, logTo(lines));
        lines.detail(built.path);
      }
      lines.end();
      return 0;
    }
    throw new HostError("sk host info|path|fetch|add|build");
  }

  if (command === "create") {
    const str = (k: string) => (typeof flags[k] === "string" ? (flags[k] as string) : undefined);
    const yesNo = (k: string) => (flags[k] ? true : flags[`no-${k}`] ? false : undefined);
    return await createCommand(args[0], {
      name: str("name"),
      os: str("os"),
      without: str("without"),
      devtools: yesNo("devtools"),
      template: str("template"),
      pm: str("pm"),
      yes: flags.yes === true,
    });
  }

  console.error(
    `${paint(colorEnabled(process.stderr)).red("error")}: unknown command ${JSON.stringify(command)} (sk --help lists them)`,
  );
  return 1;
}

if (import.meta.main) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e) => {
      const known = e instanceof ConfigError || e instanceof HostError;
      const red = paint(colorEnabled(process.stderr)).red;
      if (known || (e instanceof Error && !process.env.SK_DEBUG)) console.error(`${red("error")}: ${e.message}`);
      else console.error(e);
      process.exit(1);
    },
  );
}

export { main };
