// The local LiveKit dev server: fetch, start, stop, status.
//
//   bun devtools/lk-dev.ts fetch [--force] [--from <archive>]
//   bun devtools/lk-dev.ts start
//   bun devtools/lk-dev.ts stop
//   bun devtools/lk-dev.ts status
//
// Three rules it obeys, all of them the owner's (docs/VOICE.md §2.6, D-16):
//
//   1. LOCAL ONLY. The signalling listener is pinned to 127.0.0.1
//      (`bind_addresses` in `crates/vn-voice/tools/livekit-dev.yaml`), so the
//      only address anything can reach this server's API on is loopback.
//      **The media path is not loopback-pinned and never was** — livekit's UDP
//      mux binds one socket per interface and its ICE/TCP listener sits on
//      0.0.0.0, whatever `bind_addresses` says. This comment used to claim that
//      "every listening socket is pinned to 127.0.0.1 so Windows Firewall never
//      sees an inbound-capable bind"; `netstat` disagrees and so does
//      `netsh advfirewall`. See research/reports/m14/voice-network.md §5.
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
import { createSocket } from "node:dgram";
import { networkInterfaces } from "node:os";
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

// ---- the advertised node IP -----------------------------------------------
//
// THE FLAKE FIX (research/reports/m14/voice-network.md §3; designed as F-1 in
// m13's voice-flake.md §4).
//
// This used to pass `--node-ip 127.0.0.1`, so every ICE candidate the SFU
// signalled was `127.0.0.1:7882`. libwebrtc's port allocator drops the loopback
// adapter from the client's network list unconditionally
// (`ADAPTER_TYPE_LOOPBACK` is in `BasicNetworkManager`'s default
// `network_ignore_mask`, and livekit 0.8.2 exposes no way to clear it), so the
// client never had a loopback candidate to pair that with. The one pair it did
// form — LAN address to 127.0.0.1 — cannot work: the reply leaves the SFU's
// socket with a loopback source address on a route out of a LAN adapter and
// Windows drops it.
//
// Every green run was therefore an accident: the SFU's own connectivity check
// arrives from an address it never signalled, the client synthesises a
// PEER-REFLEXIVE candidate for it, and that pair carries the call. When the
// accident does not land inside the connect window, the run fails — which is
// the whole flake.
//
// Advertising a real local IPv4 makes the working pair the SIGNALLED one. It
// adds no exposure: byte-for-byte the same traffic every passing run already
// carried, over sockets livekit had already bound on every interface.

/** The source address the kernel would use for the default route, or null. */
async function defaultRouteIPv4(): Promise<string | null> {
  return new Promise((resolve) => {
    const sock = createSocket("udp4");
    const done = (v: string | null) => {
      try {
        sock.close();
      } catch {}
      resolve(v);
    };
    sock.on("error", () => done(null));
    // NOTHING IS SENT. `connect` on a UDP socket only fixes the peer, and that
    // is exactly what makes the kernel pick — and then expose through
    // `address()` — the source address of the route to it. 192.0.2.0/24 is
    // TEST-NET-1 (RFC 5737): unroutable on purpose, so even a stray packet
    // could not reach anything.
    try {
      sock.connect(53, "192.0.2.1", () => {
        let addr: string | null = null;
        try {
          addr = sock.address().address;
        } catch {}
        done(addr && addr !== "0.0.0.0" ? addr : null);
      });
    } catch {
      done(null);
    }
  });
}

/** Any non-internal IPv4 the OS admits to, as a second choice. */
function anyLocalIPv4(): string | null {
  for (const addrs of Object.values(networkInterfaces())) {
    for (const a of addrs ?? []) {
      if (a.family === "IPv4" && !a.internal) return a.address;
    }
  }
  return null;
}

async function nodeIp(): Promise<string> {
  const explicit = process.env.VN_LK_NODE_IP;
  if (explicit) return explicit;
  const ip = (await defaultRouteIPv4()) ?? anyLocalIPv4();
  if (ip) return ip;
  console.warn(
    "warning: no non-loopback IPv4 on this machine — advertising 127.0.0.1, which " +
      "libwebrtc's port allocator cannot pair with. Expect intermittent connect failures.",
  );
  return "127.0.0.1";
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
  const ip = await nodeIp();
  const child = spawn(exe, ["--config", CONFIG, "--node-ip", ip], {
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
      console.log(
        `livekit-server up on 127.0.0.1:7880 (pid ${child.pid}), advertising ICE candidates on ${ip}`,
      );
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
        // What a running server WOULD advertise; `start` prints what it did.
        nodeIp: await nodeIp(),
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
