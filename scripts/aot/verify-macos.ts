import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  chmodSync,
  closeSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { create } from "../../packages/cli/cli/create";
import { imageTemplatePath } from "../../packages/cli/cli/host";
import { sha256, type AotCompilerEntry, type HostManifestEntry } from "../../packages/cli/cli/host-fetch";
import { readHandshake, Wire } from "../../packages/test/src/wire";

const { values } = parseArgs({
  options: {
    hosts: { type: "string" },
    out: { type: "string" },
    "bench-apps": { type: "boolean" },
  },
});
if (process.platform !== "darwin" || process.arch !== "arm64" || !values.hosts || !values.out)
  throw Error("Run on macOS arm64 with --hosts <release-hosts> --out <scratch directory>");
const sdk = resolve(import.meta.dir, "../..");
const hosts = resolve(values.hosts);
const dir = resolve(values.out);
mkdirSync(dir, { recursive: true });
const scratch = mkdtempSync(join(dir, "macos-aot-"));
const entries = JSON.parse(readFileSync(join(hosts, "hosts-macos-arm64.json"), "utf8")) as HostManifestEntry[];
const [compiler] = JSON.parse(readFileSync(join(hosts, "aot-macos-arm64.json"), "utf8")) as AotCompilerEntry[];
assert(compiler?.platform === "macos" && compiler.arch === "arm64");

function unpack(asset: { file: string; size: number; sha256: string }, file: string) {
  assert.equal(basename(asset.file), asset.file);
  const bytes = Bun.gunzipSync(readFileSync(join(hosts, asset.file)));
  assert.equal(bytes.length, asset.size);
  assert.equal(sha256(bytes), asset.sha256);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, bytes);
  chmodSync(file, 0o755);
}
const tool = join(scratch, "sk-aot");
unpack(compiler, tool);
function hostFor(variant: string) {
  const entry = entries.find((x) => x.variant === variant);
  assert(entry?.imageTemplate, `release lacks an AOT host for ${variant}`);
  const exe = join(scratch, "hosts", variant, "sk-app");
  unpack(entry, exe);
  unpack(entry.imageTemplate, imageTemplatePath({ exe }));
  if (entry.inspector) {
    assert.equal(entry.info.aot?.inspectorContainer, "inspector.image.dylib");
    unpack(entry.inspector, join(dirname(exe), "inspector.image.dylib"));
  }
  return exe;
}

function command(
  exe: string,
  args: string[],
  env: NodeJS.ProcessEnv = {},
  expected = 0,
  timeout = 300_000,
  stdoutOnly = false,
): string {
  const result = spawnSync(exe, args, {
    cwd: sdk,
    encoding: "utf8",
    env: { ...process.env, ...env },
    timeout,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  const output = result.stdout + result.stderr;
  assert.equal(result.status, expected, `${exe} ${args.join(" ")}\n${output.slice(-12000)}`);
  return stdoutOnly ? result.stdout : output;
}
const cli = join(sdk, "packages/cli/cli/sk.ts");
const apps = new Map<string, string>();
const normal = hostFor("no-devtools");
const nojit = hostFor("aot-nojit");
const diagnostic = hostFor("aot-nojit-devtools");
const scenario = `
if (globalThis.__SK_SCENARIO__ === "aot-smoke") {
  requestAnimationFrame(() => {
    const dynamic = new Function("a", "return a + 1");
    console.log("SK_AOT_SMOKE " + dynamic(eval("40+1")));
    setTimeout(() => window.close(), 100);
  });
}
`;
function make(name: "minimal" | "showcase") {
  const root = join(scratch, "sources", name);
  create(root, { name: "probe", template: name, platforms: ["macos"] });
  const packagePath = join(root, "package.json");
  const pkg = JSON.parse(readFileSync(packagePath, "utf8")) as {
    dependencies: Record<string, string>;
    devDependencies: Record<string, string>;
    overrides?: Record<string, string>;
  };
  const localRuntime = "file:" + join(sdk, "packages/runtime");
  pkg.dependencies["@skathi-native/runtime"] = localRuntime;
  pkg.overrides = { ...pkg.overrides, "@skathi-native/runtime": localRuntime };
  pkg.devDependencies["@skathi-native/cli"] = "file:" + join(sdk, "packages/cli");
  writeFileSync(packagePath, JSON.stringify(pkg, null, 2));
  command(process.execPath, ["install", "--cwd", root]);
  const main = join(root, "src/main.ts");
  if (name === "minimal") writeFileSync(main, 'document.body.appendChild(document.createElement("div"));\n');
  else {
    copyFileSync(join(sdk, "benches/nojit/macos-scroll.ts"), join(root, "src/macos-scroll.ts"));
    appendFileSync(
      main,
      '\nimport { benchmarkScroll } from "./macos-scroll";\nvoid benchmarkScroll("main.content");\n',
    );
  }
  appendFileSync(main, scenario);
  return root;
}
function publish(source: string, label: string, host: string, aot: boolean, jit: boolean, devtools = false) {
  const base = JSON.parse(readFileSync(join(source, "sk.config.json"), "utf8")) as Record<string, unknown>;
  const cfg = join(source, label + ".config.json");
  writeFileSync(cfg, JSON.stringify({ ...base, publishJit: jit, publishDevtools: devtools }));
  const out = join(scratch, label);
  const text = command(
    process.execPath,
    [
      cli,
      "publish",
      source,
      "--out",
      out,
      "--config",
      cfg,
      "--workload",
      host,
      aot ? "--aot" : "--no-aot",
      "--aot-compiler",
      tool,
      "--json",
    ],
    {},
    0,
    900_000,
    true,
  );
  const { exe } = JSON.parse(text) as { exe: string };
  assert(existsSync(exe), exe);
  const app = dirname(dirname(dirname(exe)));
  assert(app.endsWith(".app"));
  if (aot) assert(existsSync(join(app, "Contents/Frameworks/app.image.dylib")));
  command("/usr/bin/codesign", ["--verify", "--deep", "--strict", app]);
  if (!jit) {
    const detail = command("/usr/bin/codesign", ["-d", "--entitlements", ":-", exe]);
    assert.doesNotMatch(detail, /(?:allow-jit|allow-unsigned-executable-memory|disable-library-validation)/);
  }
  apps.set(label, app);
  return { app, exe };
}
function smoke(exe: string, aot: boolean) {
  const output = command(exe, [], { SK_SCENARIO: "aot-smoke", SK_PERF: "1" }, 0, 60_000);
  assert(output.includes("SK_AOT_SMOKE 42"), output);
  assert.equal(output.includes("[native-aot] selected application image"), aot, output);
}

async function smokeInspector(exe: string): Promise<void> {
  const slot = mkdtempSync(join(scratch, "inspector-"));
  const log = join(slot, "boot.log");
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    SK_DIST: slot,
    SK_DATA_DIR: join(slot, "data"),
    SK_DEVTOOLS: "1",
    SK_DEVTOOLS_ENGINE: "auto",
    SK_DEVTOOLS_RAISE: "off",
  };
  delete env.SK_DEVTOOLS_NATIVE_IMAGE;
  delete env.SK_DEVTOOLS_TARGET;
  const fd = openSync(log, "w");
  let child;
  try {
    child = Bun.spawn([exe, "--devtools"], { cwd: sdk, env, stdin: "ignore", stdout: fd, stderr: fd });
  } finally {
    closeSync(fd);
  }
  const deadline = Date.now() + 60_000;
  const output = () => readFileSync(log, "utf8").slice(-12_000);
  let wire: Wire | undefined;
  let lastError: unknown;
  try {
    while (Date.now() < deadline && child.exitCode === null) {
      const logs = output();
      if (logs.includes("[native-aot] inspector native entry completed") && existsSync(join(slot, "devtools.json"))) {
        assert.match(
          logs,
          /\[native-aot\] selected inspector image .*\/Contents\/Frameworks\/inspector\.image\.dylib \(entry /,
        );
        if (!wire) {
          const handshake = readHandshake(slot);
          assert.equal(handshake.pid, child.pid);
          wire = new Wire(handshake, 2_000);
        }
        let tree: unknown;
        try {
          tree = await wire.send("tree", { depth: 3 });
        } catch (error) {
          lastError = error;
        }
        const dom = JSON.stringify(tree);
        if (dom?.includes("titlebar") && dom.includes("toolbar") && child.exitCode === null) {
          // Check the namespace after startup so the compiler does not record a Wasm dependency.
          const reply: unknown = await wire.send("eval", { js: "typeof WebAssembly" });
          assert.deepEqual(reply, { value: "undefined", type: "string" });
          assert.equal(child.exitCode, null, output());
          return;
        }
      }
      await Bun.sleep(100);
    }
    throw Error(
      `packaged inspector did not render its UI (exit ${child.exitCode}, ${lastError ?? "no wire error"})\n${output()}`,
    );
  } finally {
    wire?.close();
    if (child.exitCode === null) {
      child.kill("SIGTERM");
      await Promise.race([child.exited, Bun.sleep(3_000)]);
      if (child.exitCode === null) child.kill("SIGKILL");
    }
    await child.exited;
  }
}

const minimal = make("minimal");
const stock = publish(minimal, "minimal-jit", normal, false, true);
smoke(stock.exe, false);
const usual = publish(minimal, "minimal-aot", normal, true, true);
smoke(usual.exe, true);
const slim = publish(minimal, "minimal-nojit", nojit, true, false);
smoke(slim.exe, true);
const image = join(slim.app, "Contents/Frameworks/app.image.dylib");
const original = readFileSync(image);
publish(minimal, "minimal-nojit", nojit, true, false);
assert.equal(sha256(readFileSync(image)), sha256(original), "identical inputs must retain the image signature");
assert.match(command(slim.exe, [], { SK_APP_ENGINE: "jit" }, 3, 30_000), /aot-nojit.*no JIT/s);
const inspector = publish(minimal, "devtools", diagnostic, true, false, true);
smoke(inspector.exe, true);
const other = join(inspector.app, "Contents/Frameworks/inspector.image.dylib");
assert(existsSync(other));
await smokeInspector(inspector.exe);
try {
  copyFileSync(other, image);
  assert.match(command(slim.exe, [], {}, 3, 30_000), /app\.image\.dylib/);
  writeFileSync(image, "invalid Mach-O");
  assert.match(command(slim.exe, [], {}, 3, 30_000), /app\.image\.dylib/);
} finally {
  writeFileSync(image, original);
}
command("/usr/bin/codesign", ["--verify", "--deep", "--strict", slim.app]);
const assessment = spawnSync("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=4", slim.app], {
  encoding: "utf8",
});
if (assessment.error) throw assessment.error;
console.log(`Gatekeeper assessment (ad hoc) exit ${assessment.status}: ${assessment.stderr.trim()}`);
if (values["bench-apps"]) {
  const showcase = make("showcase");
  for (const [label, host, aot, jit] of [
    ["showcase-jit", normal, false, true],
    ["showcase-aot", normal, true, true],
    ["showcase-nojit", nojit, true, false],
  ] as const)
    smoke(publish(showcase, label, host, aot, jit).exe, aot);
}
writeFileSync(join(dir, "apps.json"), JSON.stringify(Object.fromEntries(apps), null, 2));
console.log(`macOS arm64 published AOT verification passed; bundles in ${scratch}`);
