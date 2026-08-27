// The local LiveKit dev server: fetch, start, stop, status.
//
//   bun devtools/lk-dev.ts fetch [--force] [--from <archive>]
//   bun devtools/lk-dev.ts start
//   bun devtools/lk-dev.ts stop
//   bun devtools/lk-dev.ts status
//
// Three rules it obeys, all of them the owner's (docs/VOICE.md §2.6, D-16):
//
//   1. LOOPBACK ONLY. `tools/livekit/livekit-dev.yaml` pins every bind to
//      127.0.0.1, so Windows Firewall never raises a prompt.
//   2. ON DEMAND. Started by a test or by a human, never by `vn-app`'s boot.
//   3. NEVER LEFT RUNNING. The integration harness stops it in a `finally`,
//      pass or fail.
//
// `start` POLLS `http://127.0.0.1:7880/` until it answers rather than sleeping
// a fixed amount — the same discipline the devtools listener follows. A sleep
// is either too short (flaky) or too long (slow), and it is never right.
//
// Dependency-free on purpose, like ctl.ts: an agent must be able to drive this
// through Bash without `bun install` having run.

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readdirSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const REPO = resolve(import.meta.dir, "../../..");
// SOURCE lives beside the crate; the FETCHED binary and this run's scratch live
// under /tools, which .gitignore excludes wholesale. Same split as
// crates/vn-effects/tools/slang.manifest.json versus tools/slang/.
const SRC = join(REPO, "crates/vn-voice/tools");
const MANIFEST = join(SRC, "livekit.manifest.json");
const CONFIG = join(SRC, "livekit-dev.yaml");
const RUN = join(REPO, "tools/livekit");
const BIN_DIR = join(RUN, "bin");
const PID_FILE = join(RUN, "livekit.pid");
const OUT_LOG = join(RUN, "livekit.out.log");
const ERR_LOG = join(RUN, "livekit.err.log");

const HEALTH = "http://127.0.0.1:7880/";
/** How long `start` waits for the server to answer before giving up. */
const START_TIMEOUT_MS = 20_000;
const POLL_INTERVAL_MS = 100;

type Asset = { url: string; sha256: string; exe: string };
type Manifest = { tool: string; version: string; assets: Record<string, Asset> };

function manifest(): Manifest {
  if (!existsSync(MANIFEST)) throw new Error(`missing ${MANIFEST}`);
  return JSON.parse(readFileSync(MANIFEST, "utf8"));
}

function platformKey(): string {
  const key = `${process.platform}-${process.arch}`;
  const m = manifest();
  if (!m.assets[key]) {
    throw new Error(
      `no pinned livekit-server for ${key}. Add it to ${MANIFEST} — the checksums for ` +
        `every platform are published at the release's checksums.txt.`,
    );
  }
  return key;
}

function exePath(): string {
  return join(BIN_DIR, manifest().assets[platformKey()].exe);
}

// ---- fetch ----------------------------------------------------------------

async function fetchServer(force: boolean, from: string | null) {
  const m = manifest();
  const asset = m.assets[platformKey()];
  const exe = exePath();
  if (existsSync(exe) && !force) {
    console.log(`livekit-server ${m.version} already at ${exe} (use --force to refetch)`);
    return;
  }

  const tmp = mkdtempSync(join(tmpdir(), "lk-"));
  try {
    // The extension is load-bearing: `Expand-Archive` refuses anything not
    // named `.zip`, however well-formed the bytes are.
    const isZip = asset.url.endsWith(".zip");
    const archive = join(tmp, isZip ? "livekit.zip" : "livekit.tar.gz");
    if (from) {
      // Escape hatch for an already-downloaded archive. It is verified against
      // the SAME pin as a download, so this can shorten the wait but never
      // weaken the check.
      copyFileSync(resolve(from), archive);
      console.log(`using local archive ${from}`);
    } else {
      console.log(`downloading ${asset.url}`);
      const response = await fetch(asset.url);
      if (!response.ok) throw new Error(`${asset.url} responded ${response.status}`);
      writeFileSync(archive, Buffer.from(await response.arrayBuffer()));
    }

    // Verified BEFORE anything is extracted: an archive that fails this is
    // never written anywhere a later step could pick it up.
    const actual = createHash("sha256").update(readFileSync(archive)).digest("hex");
    if (actual !== asset.sha256) {
      throw new Error(
        `sha256 mismatch for ${asset.url}\n  expected ${asset.sha256}\n  actual   ${actual}`,
      );
    }
    console.log(`sha256 ok (${actual})`);

    const unpack = join(tmp, "unpacked");
    mkdirSync(unpack, { recursive: true });
    const result = isZip
      ? spawnSync(
          "powershell",
          ["-NoProfile", "-Command", `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${unpack}' -Force`],
          { stdio: "inherit" },
        )
      : spawnSync("tar", ["-xzf", archive, "-C", unpack], { stdio: "inherit" });
    if (result.status !== 0) throw new Error("could not unpack the archive");

    const found = readdirSync(unpack).find((f) => f === asset.exe);
    if (!found) throw new Error(`${asset.exe} not found in the archive`);
    mkdirSync(BIN_DIR, { recursive: true });
    copyFileSync(join(unpack, found), exe);
    if (process.platform !== "win32") chmodSync(exe, 0o755);
    console.log(`livekit-server ${m.version} -> ${exe}`);
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

// ---- lifecycle ------------------------------------------------------------

async function healthy(): Promise<boolean> {
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 1000);
    const response = await fetch(HEALTH, { signal: controller.signal });
    clearTimeout(timer);
    // Any HTTP answer means the listener is up; livekit-server returns 200
    // with "OK" on `/`.
    return response.status > 0;
  } catch {
    return false;
  }
}

function readPid(): number | null {
  if (!existsSync(PID_FILE)) return null;
  const pid = Number.parseInt(readFileSync(PID_FILE, "utf8").trim(), 10);
  return Number.isFinite(pid) && pid > 0 ? pid : null;
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function start() {
  if (await healthy()) {
    console.log("livekit-server is already listening on 127.0.0.1:7880");
    return;
  }
  const exe = exePath();
  if (!existsSync(exe)) {
    throw new Error(`${exe} is missing — run \`bun devtools/lk-dev.ts fetch\` first`);
  }

  // Detached, with no console window, so it does not steal focus or die with
  // this shell. The owner is using this machine.
  //
  // The log files are passed as FILE DESCRIPTORS rather than piped: a piped
  // stdio keeps a handle open on THIS process's event loop, so `start` would
  // never return even after the server answered. Handing the child raw fds
  // means the parent holds nothing and can exit the moment the poll succeeds.
  mkdirSync(RUN, { recursive: true });
  const out = openSync(OUT_LOG, "a");
  const err = openSync(ERR_LOG, "a");
  const child = spawn(exe, ["--config", CONFIG, "--node-ip", "127.0.0.1"], {
    detached: true,
    windowsHide: true,
    stdio: ["ignore", out, err],
  });
  child.unref();
  writeFileSync(PID_FILE, String(child.pid));

  // POLL, do not sleep.
  const deadline = Date.now() + START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await healthy()) {
      console.log(`livekit-server up on 127.0.0.1:7880 (pid ${child.pid})`);
      return;
    }
    if (child.pid && !alive(child.pid)) break;
    await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
  }
  const tail = existsSync(ERR_LOG) ? readFileSync(ERR_LOG, "utf8").split("\n").slice(-20).join("\n") : "";
  await stop();
  throw new Error(`livekit-server did not come up within ${START_TIMEOUT_MS} ms\n${tail}`);
}

async function stop() {
  const pid = readPid();
  if (pid && alive(pid)) {
    if (process.platform === "win32") {
      spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
    } else {
      try {
        process.kill(pid, "SIGTERM");
      } catch {}
    }
    console.log(`stopped livekit-server (pid ${pid})`);
  } else {
    console.log("livekit-server is not running");
  }
  if (existsSync(PID_FILE)) unlinkSync(PID_FILE);
}

async function status() {
  const pid = readPid();
  const up = await healthy();
  console.log(
    JSON.stringify(
      {
        version: manifest().version,
        installed: existsSync(exePath()),
        exe: exePath(),
        pid: pid && alive(pid) ? pid : null,
        listening: up,
        url: "ws://127.0.0.1:7880",
      },
      null,
      2,
    ),
  );
  if (!up) process.exitCode = 1;
}

// ---- main -----------------------------------------------------------------

const [command, ...rest] = process.argv.slice(2);
const fromIndex = rest.indexOf("--from");

try {
  switch (command) {
    case "fetch":
      await fetchServer(rest.includes("--force"), fromIndex >= 0 ? rest[fromIndex + 1] : null);
      break;
    case "start":
      await start();
      break;
    case "stop":
      await stop();
      break;
    case "status":
      await status();
      break;
    default:
      console.log("usage: bun devtools/lk-dev.ts <fetch|start|stop|status> [--force] [--from <archive>]");
      process.exitCode = 2;
  }
} catch (e) {
  console.error(String(e instanceof Error ? e.message : e));
  process.exitCode = 1;
}
