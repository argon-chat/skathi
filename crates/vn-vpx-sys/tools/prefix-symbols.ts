#!/usr/bin/env bun
/**
 * Regenerates `crates/vn-vpx-sys/config/prefix/<configuration>.h`: a
 * `#define NAME vnvpx_NAME` for every symbol this crate's libvpx defines that
 * another libvpx in the same link would define too. build.rs force-includes
 * the header into every C file it compiles, so the crate's libvpx links next
 * to libwebrtc's (webrtc.lib bundles Chromium's libvpx: `vpx_malloc`,
 * `vpx_codec_decode`, `vp9_*` and the rest would otherwise be defined twice,
 * LNK2005), and the two never call into each other.
 *
 *     bun crates/vn-vpx-sys/tools/prefix-symbols.ts           # regenerate, then verify
 *     bun crates/vn-vpx-sys/tools/prefix-symbols.ts --check   # verify only
 *
 * Windows with MSVC only (it reads the objects with `dumpbin`; `DUMPBIN=` to
 * point at one). It runs cargo, so mind the one-build-at-a-time rule.
 *
 * For each configuration (`x86_64-msvc`, and `generic` built by MSVC: the same
 * C files define the same symbols whatever compiles them):
 *  1. `cargo build -p vn-vpx-sys` with `VN_VPX_PREFIX=0` (no header) into
 *     `target/vn-vpx-symbols/<configuration>`, and `dumpbin /symbols` on the
 *     static library;
 *  2. the symbols to rename: every external symbol defined outside a COMDAT
 *     or in a "no duplicates" one (`/Gy` puts each function in one): those
 *     are LNK2005 next to another copy. And every common symbol (an
 *     uninitialised global, such as each rtcd function pointer): the linker
 *     merges those silently, so two libvpx would share them, worse than a
 *     link error. Not the other COMDATs ("pick any"...: inline CRT functions
 *     and their statics, SIMD constants, string literals), which are the
 *     same bytes whoever brings them; nor the crate's own `vn_vpx_*`;
 *  3. writes the header;
 *  4. builds again with it and checks that every such symbol now starts with
 *     `vnvpx_` (or is the crate's `vn_vpx_*`).
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const crateDir = resolve(import.meta.dir, "..");
const repo = resolve(crateDir, "..", "..");
const CONFIGS = ["x86_64-msvc", "generic"];
const PREFIX = "vnvpx_";
const OWN = "vn_vpx_";

function die(message: string): never {
  console.error(`prefix-symbols: ${message}`);
  process.exit(1);
}

function dumpbin(): string {
  if (process.env.DUMPBIN) return process.env.DUMPBIN;
  const vswhere = "C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe";
  if (!existsSync(vswhere)) die("no vswhere.exe; set DUMPBIN=<path to dumpbin.exe>");
  const installs = Bun.spawnSync([vswhere, "-all", "-products", "*", "-property", "installationPath"]).stdout.toString().split(/\r?\n/).filter(Boolean);
  const found: [string, string][] = [];
  for (const install of installs) {
    const tools = join(install, "VC", "Tools", "MSVC");
    if (!existsSync(tools)) continue;
    for (const version of readdirSync(tools)) {
      const exe = join(tools, version, "bin", "Hostx64", "x64", "dumpbin.exe");
      if (existsSync(exe)) found.push([version, exe]);
    }
  }
  const versionKey = (v: string) => v.split(".").map((n) => n.padStart(6, "0")).join(".");
  found.sort((a, b) => versionKey(b[0]).localeCompare(versionKey(a[0])));
  return found[0]?.[1] ?? die("no dumpbin.exe under Visual Studio; set DUMPBIN=");
}

/** Builds the crate's static library in `config`, with or without the header. */
function build(config: string, prefixed: boolean): string {
  const target = join(repo, "target", "vn-vpx-symbols", config);
  const env = { ...process.env, VN_VPX_CONFIG: config, CARGO_TARGET_DIR: target, VN_VPX_PREFIX: prefixed ? "1" : "0" };
  console.log(`cargo build -p vn-vpx-sys (${config}, ${prefixed ? "prefixed" : "not prefixed"})`);
  const cargo = Bun.spawnSync(["cargo", "build", "-p", "vn-vpx-sys", "--quiet"], { cwd: repo, env, stdout: "inherit", stderr: "inherit" });
  if (cargo.exitCode !== 0) die(`cargo build failed (${config})`);
  const libs: string[] = [];
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name === "vn_vpx.lib" && p.includes("vn-vpx-sys-")) libs.push(p);
    }
  };
  walk(target);
  libs.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  return libs[0] ?? die(`no vn_vpx.lib under ${target}`);
}

/**
 * The library's symbols that would collide with another copy: externals
 * defined outside a COMDAT or in a "no duplicates" one, and commons.
 */
function colliding(lib: string, exe: string): Set<string> {
  const run = Bun.spawnSync([exe, "/nologo", "/symbols", lib]);
  if (run.exitCode !== 0) die(`dumpbin failed on ${lib}`);
  const out = new Set<string>();
  // Per object: section number -> COMDAT selection (absent: not a COMDAT),
  // and the externals defined in each section, resolved at the object's end.
  let selection = new Map<string, number>();
  let defined: [string, string][] = [];
  let lastSection: string | null = null;
  const flush = () => {
    for (const [name, section] of defined) {
      const s = selection.get(section);
      if (s === undefined || s === 1) out.add(name);
    }
    selection = new Map();
    defined = [];
  };
  const symbol = /^[0-9A-F]{3,} ([0-9A-F]{8}) (SECT[0-9A-F]+|UNDEF|ABS|DEBUG)\s+\S+(?: \(\))?\s+(\w+)\s+\| (\S+)/;
  for (const line of run.stdout.toString().split(/\r?\n/)) {
    if (line.includes("COFF SYMBOL TABLE")) {
      flush();
      continue;
    }
    const aux = /selection\s+(\d+)/.exec(line);
    if (aux && lastSection && /^\s+Section length/.test(line)) {
      selection.set(lastSection, Number(aux[1]));
      continue;
    }
    const m = symbol.exec(line);
    if (!m) continue;
    const [, value, section, storage, name] = m;
    lastSection = storage === "Static" && section.startsWith("SECT") ? section : null;
    if (storage !== "External") continue;
    if (section.startsWith("SECT")) defined.push([name, section]);
    // A common: the linker merges it with every other of the same name, with
    // no error, so two libvpx would silently share it (an rtcd pointer set
    // by one, called through by the other). Those are C identifiers; a
    // C++-mangled one is a CRT inline function's static.
    else if (section === "UNDEF" && value !== "00000000" && !name.startsWith("?")) out.add(name);
  }
  flush();
  return out;
}

function header(config: string, names: string[]): string {
  const lines = [
    `/* Generated by tools/prefix-symbols.ts for config/${config}/: do not edit. */`,
    `/* Every symbol this configuration's libvpx defines that another libvpx in`,
    `   the same link (libwebrtc's) would define too, renamed to ${PREFIX}*.`,
    `   build.rs force-includes this file into every C file it compiles; the`,
    `   Rust bindings link to the ${PREFIX}* names. VENDOR.md, "Symbol prefix". */`,
    `#ifndef VN_VPX_PREFIX_H_`,
    `#define VN_VPX_PREFIX_H_`,
    ...names.map((n) => `#define ${n} ${PREFIX}${n}`),
    `#endif  /* VN_VPX_PREFIX_H_ */`,
    ``,
  ];
  return lines.join("\n");
}

const check = process.argv.includes("--check");
const exe = dumpbin();
let failed = false;
for (const config of CONFIGS) {
  const file = join(crateDir, "config", "prefix", `${config}.h`);
  if (!check) {
    const raw = [...colliding(build(config, false), exe)];
    const own = raw.filter((n) => n.startsWith(OWN));
    const names = raw.filter((n) => !n.startsWith(OWN)).sort();
    const bad = names.filter((n) => !/^[A-Za-z_][A-Za-z0-9_]*$/.test(n) || n.startsWith(PREFIX));
    if (bad.length) die(`symbols a #define cannot rename (${config}): ${bad.join(", ")}`);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, header(config, names));
    console.log(`config/prefix/${config}.h: ${names.length} symbols (and ${own.length} of the crate's own ${OWN}*, kept)`);
  }
  if (!existsSync(file)) die(`no ${file}; run without --check`);
  const listed = readFileSync(file, "utf8").split("\n").filter((l) => l.startsWith("#define ") && !l.includes("VN_VPX_PREFIX_H_")).length;
  const after = [...colliding(build(config, true), exe)];
  const unprefixed = after.filter((n) => !n.startsWith(PREFIX) && !n.startsWith(OWN)).sort();
  const prefixed = after.filter((n) => n.startsWith(PREFIX)).length;
  if (unprefixed.length) {
    console.error(`${config}: ${unprefixed.length} symbols not prefixed: ${unprefixed.join(", ")}`);
    failed = true;
  } else {
    console.log(`${config}: ${prefixed} symbols, all ${PREFIX}* (the header lists ${listed}); no unprefixed global symbol`);
  }
}
if (failed) die("the headers are stale: run without --check");
