#!/usr/bin/env bun
// vn — create, develop and package a vue-native app as a Windows exe or a macOS .app, without a
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
import { create } from "./create";
import { currentPlatform, parsePlatform } from "./platform";

const USAGE = `vn ${CLI_VERSION} — package a vue-native app as a Windows exe or a macOS .app

usage
  vn dev     [app] [--exe <h>] the dev loop: build.ts --watch + a dev host (hot reload); the
                               host is the checkout's debug vn-app, else this version's,
                               cached or downloaded from its release
  vn build   [app] [options]   a debug exe in <app>/dist/debug: devtools (F12) in, the bundle
                               not minified
  vn publish [app] [options]   the release exe in <app>/dist/release: the engine parts
                               vn.config.json selects, and without a selection no devtools;
                               minified (macOS: <exeName>.app, signed per macos.sign)
  vn run     [app] [options]   vn build, then start the exe with its stderr in this terminal
  vn host info [exe]           what a host exe says about itself (--vn-host-info)
  vn host path [selection]     which host \`vn build\` would stamp
  vn host fetch [selection | --variant both | --variant dev]
                               download prebuilt hosts of this version from its GitHub
                               release into the local cache (no selection: every published
                               one and the dev host; vn build and vn dev do it on demand)
  vn host add <exe>            verify a prod host and copy it into the local cache
  vn host build [selection | --variant both]
                               maintainers, in a checkout: cargo-build the bare host(s)
                               (no selection: standard and no-devtools)
  vn create <dir> [--name n]   a new app: vn.config.json, src/main.ts, src/App.vue, assets/

  [app] is an app directory (default: .), its entry module, or its vn.config.json.

build and publish options
  --out <dir>                  output directory (default <app>/dist/debug or dist/release)
  --config <path>              external config; entry and relative assets stay under [app]
  --host <exe>                 the bare host to stamp (default: VN_HOST, the checkout's
                               target/…/release/vn-app.exe, the local cache, then a
                               download from the release; VN_HOST_OFFLINE=1 skips that)
  selection — which engine parts the host carries (default: vn.config.json
  "windows": "without" / "features" / "host", else every part; vn build always keeps
  devtools, vn publish drops them unless the config selects them). One of:
  --without audio,clipboard    leave these parts out
  --features audio,storages    take only these parts
  --variant no-devtools+no-audio
                               a host variant by name ("standard" = every part)
  parts: ${OPTIONAL.join(", ")}
  --console                    keep the console subsystem (a console window opens with it)
  --platform windows|macos     what to package for (default: this OS). Another OS's
                               build needs --host <its vn-app>, and is not signed
  --sign <identity>            macOS: codesign with this identity, hardened runtime and
                               entitlements ("-" = ad hoc; over vn.config.json macos.sign)
  --no-sign                    macOS: leave the bundle unsigned (default: ad hoc)
  --payload-only               write dist/.vn/app.vnpk and stop; no host needed
  --aot-inputs                 retain typed TS/Vue source inputs in dist/.vn/aot-inputs/
                               (preparation archive; does not enable native AOT)
  --allow-host-mismatch        stamp a host from another vue-native version (maintainers)
  --define K=V                 a Vite-style define on top of vn.config.json "define"
                               (V is an expression: --define __FLAG__=true,
                               --define 'APP_NAME="argon"'); repeatable
  --json                       one JSON line instead of the summary (build, host path)

  vn.config.json "packs" splits the assets into <name>.vnpk files beside the payload
  (or the exe), e.g. [{ "name": "fonts", "match": ["**/*.woff2"] }, { "name": "assets" }].
`;

type Flags = Record<string, string | boolean>;
const BOOLEAN = new Set(["console", "payload-only", "allow-host-mismatch", "aot-inputs", "help", "json", "no-sign"]);
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
  if (!command || flags.help || command === "help") {
    console.log(USAGE);
    return command || flags.help ? 0 : 1;
  }
  if (command === "version" || command === "--version") {
    console.log(CLI_VERSION);
    return 0;
  }

  if (command === "build" || command === "publish" || command === "run") {
    const platform = flags.platform === undefined ? undefined : parsePlatform(flags.platform);
    if (command === "run" && platform && platform !== currentPlatform()) {
      throw new ConfigError(
        `vn run starts the app here; a ${platform} build cannot run on ${process.platform} — use vn build`,
      );
    }
    if (flags.sign !== undefined && (typeof flags.sign !== "string" || !flags.sign.trim())) {
      throw new ConfigError('--sign needs a codesign identity ("-" for ad hoc)');
    }
    if (flags.sign !== undefined && flags["no-sign"])
      throw new ConfigError("--sign and --no-sign contradict each other");
    const result = await buildApp({
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
    });
    if (flags.json) {
      const s = result.stamp;
      console.log(
        JSON.stringify({
          exe: result.exe,
          vnpk: result.vnpk,
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
      console.log(summary(result));
    }
    if (command !== "run" || !result.exe) return 0;
    // `run`: inherited handles, so a GUI-subsystem exe's stderr still lands in this terminal.
    const child = spawn(result.exe, rest, { stdio: "inherit" });
    return await new Promise<number>((done) => child.on("exit", (c) => done(c ?? 0)));
  }

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
      if (!arg) throw new HostError("vn host add <exe>");
      addHost(arg, console.log);
      return 0;
    }
    if (sub === "fetch") {
      const named = flags.variant === "both" || flags.variant === DEV_HOST ? flags.variant : variantOf(flags);
      const variants = named === undefined ? [...VARIANTS, DEV_HOST] : named === "both" ? VARIANTS : [named];
      for (const variant of variants) await fetchHost({ variant, log: console.log });
      return 0;
    }
    if (sub === "build") {
      const v = flags.variant === "both" ? "both" : (variantOf(flags) ?? "both");
      for (const variant of v === "both" ? VARIANTS : [v]) {
        const built = buildHost(variant);
        console.log(`[vn] ${variant}: ${built.path} (cargo ${(built.ms / 1000).toFixed(0)} s)`);
      }
      return 0;
    }
    throw new HostError("vn host info|path|fetch|add|build");
  }

  if (command === "create") {
    if (!args[0]) throw new ConfigError("vn create <dir>");
    create(args[0], { name: typeof flags.name === "string" ? flags.name : undefined });
    return 0;
  }

  console.error(`unknown command ${JSON.stringify(command)}\n\n${USAGE}`);
  return 1;
}

if (import.meta.main) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (e) => {
      const known = e instanceof ConfigError || e instanceof HostError;
      console.error(known ? `vn: ${e.message}` : e);
      process.exit(1);
    },
  );
}

export { main };
