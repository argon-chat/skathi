#!/usr/bin/env bun
/**
 * Re-vendors libvpx's VP9 decoder subset into `crates/vn-vpx-sys/libvpx/` and
 * regenerates the build headers under `crates/vn-vpx-sys/config/`.
 *
 *     bun crates/vn-vpx-sys/tools/vendor-libvpx.ts [--src <extracted libvpx-1.17.0>]
 *
 * Without `--src` it downloads the pinned release tarball and checks its
 * sha256. Run it from Git Bash (or any shell where `bash` is a POSIX bash, not
 * WSL's): it runs libvpx's `configure` (bash), `build/make/rtcd.pl` (perl) and
 * `build/make/version.sh` (sh). The generic configuration's `configure` probes
 * a C compiler; `clang` on PATH is enough (no gcc or make is needed).
 *
 * What it does, in order (VENDOR.md explains why):
 *  1. `configure` twice, into temp dirs: `x86_64-win64-vs17` (the intrinsics
 *     build) and `generic-gnu` (C only, every other target).
 *  2. `rtcd.pl` for vp9_rtcd.h, vpx_dsp_rtcd.h and vpx_scale_rtcd.h per
 *     configuration, and `version.sh` for vpx_version.h.
 *  3. The no-nasm patch: every entry of `config/asm-only.txt` switched to C in
 *     the x86_64 headers. Fails if an entry does not match exactly one line.
 *  4. Copies `sources.txt`'s C files, every header they reach through
 *     `#include "..."` (other architectures' headers excepted), and the
 *     licence files into a fresh `libvpx/`. Nothing in it is edited.
 *
 * The output is deterministic: re-running it on the same release must leave `git status` clean.
 */
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";

const VERSION = "1.17.0";
const RELEASE_URL = `https://github.com/webmproject/libvpx/archive/refs/tags/v${VERSION}.tar.gz`;
const SHA256 = "1020f184046187baa2985dbde38e0691f49c44088bca7a1842b0236c6081dc0a";

/** Flags both configurations share: the VP9 decoder and nothing else. */
const COMMON_FLAGS = [
  "--disable-vp8",
  "--disable-vp9-encoder",
  "--enable-vp9-decoder",
  "--disable-examples",
  "--disable-tools",
  "--disable-docs",
  "--disable-unit-tests",
  "--disable-webm-io",
  "--disable-libyuv",
  "--disable-postproc",
  "--disable-vp9-postproc",
  // One thread per decoder (the sticker worker pool is the parallelism). It
  // also makes libvpx's `once()` a plain flag: src/lib.rs runs the first
  // decoder's set-up under a std::sync::Once for that reason.
  "--disable-multithread",
  // The sticker contract's "larger than 4096 px is unsupported", enforced by
  // the decoder itself: a frame header claiming more fails before allocating.
  "--size-limit=4096x4096",
];

interface Config {
  /** Directory under config/. */
  name: string;
  target: string;
  /** rtcd.pl --arch. */
  arch: string;
  flags: string[];
  env?: Record<string, string>;
}

const CONFIGS: Config[] = [
  {
    name: "x86_64-msvc",
    target: "x86_64-win64-vs17",
    arch: "x86_64",
    // The SIMD set configure picks for x86_64 (SSE2..AVX2; MSVC gets no
    // AVX-512 from libvpx), runtime CPU detection on. `--as=nasm` only
    // satisfies configure: no .asm file is assembled.
    flags: ["--as=nasm", "--enable-static-msvcrt"],
  },
  {
    name: "generic",
    target: "generic-gnu",
    arch: "generic",
    flags: [],
    // configure only probes that a compiler runs; clang stands in for gcc.
    env: { CC: "clang", CXX: "clang++", LD: "clang", AR: "llvm-ar" },
  },
];

const RTCD: [sym: string, defs: string][] = [
  ["vp9_rtcd", "vp9/common/vp9_rtcd_defs.pl"],
  ["vpx_dsp_rtcd", "vpx_dsp/vpx_dsp_rtcd_defs.pl"],
  ["vpx_scale_rtcd", "vpx_scale/vpx_scale_rtcd.pl"],
];

/** Headers of other architectures: reachable only under their `#if`s. */
const FOREIGN = /(^|\/)(arm|mips|ppc|loongarch)(\/|$)|(^|\/)vpx_ports\/(arm|arm_cpudetect|mips|ppc|loongarch|aarch64_cpudetect)\b/;

const LICENCE_FILES = ["LICENSE", "PATENTS", "AUTHORS"];

const crateDir = resolve(import.meta.dir, "..");

function die(msg: string): never {
  console.error(`vendor-libvpx: ${msg}`);
  process.exit(1);
}

function run(cmd: string[], cwd: string, env?: Record<string, string>): string {
  const p = Bun.spawnSync(cmd, { cwd, env: { ...process.env, ...env }, stdout: "pipe", stderr: "pipe" });
  if (p.exitCode !== 0) {
    die(`${cmd.join(" ")} (in ${cwd}) exited ${p.exitCode}\n${p.stdout.toString()}\n${p.stderr.toString()}`);
  }
  return p.stdout.toString();
}

/** Forward slashes, so bash and perl take Windows paths as they are. */
const slash = (p: string) => p.replace(/\\/g, "/");

async function fetchRelease(work: string): Promise<string> {
  console.log(`fetching ${RELEASE_URL}`);
  const res = await fetch(RELEASE_URL);
  if (!res.ok) die(`download failed: ${res.status} ${res.statusText}`);
  const bytes = new Uint8Array(await res.arrayBuffer());
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== SHA256) die(`sha256 mismatch for ${RELEASE_URL}\n  expected ${SHA256}\n  actual   ${actual}`);
  console.log(`sha256 ok: ${actual}`);
  const tarball = join(work, "libvpx.tar.gz");
  writeFileSync(tarball, bytes);
  run(["tar", "-xzf", "libvpx.tar.gz"], work);
  return join(work, `libvpx-${VERSION}`);
}

function lf(text: string): string {
  return text.replace(/\r\n/g, "\n");
}

/** Step 3: the asm-only entries, switched to C. Returns the patched headers. */
function patchAsmOnly(headers: Map<string, string>): Map<string, string> {
  const list = lf(readFileSync(join(crateDir, "config/asm-only.txt"), "utf8"))
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
  const hits = new Map<string, number>(list.map((s) => [s, 0]));
  const out = new Map<string, string>();
  for (const [name, text] of headers) {
    const lines: string[] = [];
    let patched = 0;
    for (const line of text.split("\n")) {
      const def = /^#define (\w+) (\w+)$/.exec(line);
      if (def && hits.has(def[2])) {
        hits.set(def[2], hits.get(def[2])! + 1);
        lines.push(`#define ${def[1]} ${def[1]}_c`);
        patched++;
        continue;
      }
      const dispatch = /^\s*if \(flags & HAS_\w+\) (\w+) = (\w+);$/.exec(line);
      if (dispatch && hits.has(dispatch[2])) {
        hits.set(dispatch[2], hits.get(dispatch[2])! + 1);
        patched++;
        continue;
      }
      lines.push(line);
    }
    let result = lines.join("\n");
    if (patched) {
      result = result.replace(
        "// This file is generated. Do not edit.\n",
        "// This file is generated. Do not edit.\n" +
          `// vn-vpx-sys: ${patched} asm-only specialisations switched to C by\n` +
          "// tools/vendor-libvpx.ts (config/asm-only.txt): no nasm in this build.\n",
      );
    }
    out.set(name, result);
  }
  const wrong = [...hits].filter(([, n]) => n !== 1);
  if (wrong.length) {
    die(`config/asm-only.txt is out of date:\n${wrong.map(([s, n]) => `  ${s}: matched ${n} lines`).join("\n")}`);
  }
  return out;
}

/** Steps 1-3 for one configuration. Returns header name -> text. */
function generate(src: string, cfg: Config, work: string): Map<string, string> {
  const dir = join(work, `cfg-${cfg.name}`);
  mkdirSync(dir, { recursive: true });
  const configure = slash(relative(dir, join(src, "configure")));
  console.log(`configure --target=${cfg.target}`);
  run(["bash", configure, `--target=${cfg.target}`, ...COMMON_FLAGS, ...cfg.flags], dir, cfg.env);
  const mk = `libs-${cfg.target}.mk`;
  const options = /^RTCD_OPTIONS = (.*)$/m.exec(lf(readFileSync(join(dir, mk), "utf8")))?.[1].trim().split(/\s+/).filter(Boolean) ?? [];
  const headers = new Map<string, string>();
  headers.set("vpx_config.h", lf(readFileSync(join(dir, "vpx_config.h"), "utf8")));
  for (const [sym, defs] of RTCD) {
    const text = run(
      ["perl", slash(join(src, "build/make/rtcd.pl")), `--arch=${cfg.arch}`, `--sym=${sym}`, `--config=${mk}`, ...options, slash(join(src, defs))],
      dir,
    );
    headers.set(`${sym}.h`, lf(text));
  }
  return cfg.arch === "x86_64" ? patchAsmOnly(headers) : headers;
}

function sourceList(): { all: string[] } {
  const all: string[] = [];
  for (const raw of lf(readFileSync(join(crateDir, "sources.txt"), "utf8")).split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("[")) continue;
    all.push(line);
  }
  return { all };
}

/**
 * Step 4's closure: every libvpx header a listed file reaches through
 * `#include "..."`, resolved as the compiler does (the including file's
 * directory, then the libvpx root). The generated headers (config/) are not
 * copied, but what they include is; other architectures' headers are left out.
 */
function headerClosure(src: string, files: string[], generated: Map<string, string>): string[] {
  const seen = new Set<string>();
  const queue: string[] = [];
  const headers: string[] = [];
  const follow = (from: string, text: string) => {
    for (const m of text.matchAll(/^\s*#\s*include\s+"([^"]+)"/gm)) {
      const inc = m[1];
      const base = inc.replace(/^\.\//, "");
      if (generated.has(base)) continue;
      const local = slash(join(dirname(from), inc)).replace(/^\.\//, "");
      const candidate = existsSync(join(src, local)) ? local : existsSync(join(src, base)) ? slash(base) : null;
      if (!candidate) die(`${from}: cannot resolve #include "${inc}"`);
      if (FOREIGN.test(candidate)) continue;
      queue.push(candidate);
    }
  };
  // A generated header lives at the include root, so its includes resolve
  // against the libvpx root.
  for (const [name, text] of generated) follow(name, text);
  queue.push(...files);
  while (queue.length) {
    const file = queue.pop()!;
    if (seen.has(file)) continue;
    seen.add(file);
    if (!files.includes(file)) headers.push(file);
    follow(file, readFileSync(join(src, file), "utf8"));
  }
  return headers.sort();
}

function sizeOf(dir: string): [files: number, bytes: number] {
  let files = 0;
  let bytes = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      const [f, b] = sizeOf(p);
      files += f;
      bytes += b;
    } else {
      files++;
      bytes += statSync(p).size;
    }
  }
  return [files, bytes];
}

async function main() {
  const argv = process.argv.slice(2);
  const work = mkdtempSync(join(tmpdir(), "vendor-libvpx-"));
  try {
    const srcArg = argv.indexOf("--src");
    const src = srcArg >= 0 ? resolve(argv[srcArg + 1] ?? die("--src needs a directory")) : await fetchRelease(work);
    const changelog = lf(readFileSync(join(src, "CHANGELOG"), "utf8"));
    if (!changelog.includes(`v${VERSION}`)) die(`${src} is not libvpx ${VERSION} (CHANGELOG)`);

    // Steps 1-3. `generated` keeps every configuration's text of each header,
    // for step 4's include scan.
    const generated = new Map<string, string>();
    for (const cfg of CONFIGS) {
      const headers = generate(src, cfg, work);
      const out = join(crateDir, "config", cfg.name);
      rmSync(out, { recursive: true, force: true });
      mkdirSync(out, { recursive: true });
      for (const [name, text] of headers) {
        writeFileSync(join(out, name), text);
        generated.set(name, (generated.get(name) ?? "") + text);
      }
    }
    run(["sh", slash(join(src, "build/make/version.sh")), slash(src), "vpx_version.h"], work);
    const version = lf(readFileSync(join(work, "vpx_version.h"), "utf8"));
    if (!version.includes(`"v${VERSION}"`)) die(`version.sh made a vpx_version.h for another version:\n${version}`);
    writeFileSync(join(crateDir, "config", "vpx_version.h"), version);
    generated.set("vpx_version.h", version);

    // Step 4.
    const { all } = sourceList();
    for (const f of all) if (!existsSync(join(src, f))) die(`sources.txt: ${f} is not in libvpx ${VERSION}`);
    const headers = headerClosure(src, all, generated);
    const dest = join(crateDir, "libvpx");
    rmSync(dest, { recursive: true, force: true });
    for (const f of [...all, ...headers, ...LICENCE_FILES]) {
      const to = join(dest, f);
      mkdirSync(dirname(to), { recursive: true });
      cpSync(join(src, f), to);
    }
    const [files, bytes] = sizeOf(dest);
    console.log(`libvpx/: ${all.length} sources, ${headers.length} headers, ${LICENCE_FILES.length} licence files (${files} files, ${(bytes / 1024).toFixed(0)} KiB)`);
    console.log("next: bun crates/vn-vpx-sys/tools/prefix-symbols.ts (the symbol prefix, VENDOR.md)");
  } finally {
    if (!argv.includes("--keep-temp")) rmSync(work, { recursive: true, force: true });
    else console.log(`temp kept: ${work}`);
  }
}

await main();
