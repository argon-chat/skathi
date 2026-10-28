// `launch()` and the `VnApp` handle — the browser-and-page half of the API.
//
// Playwright splits a run into browser / context / page. We have one window per
// process and no contexts, so `VnApp` is all three at once: it owns the OS
// process, the dist slot it was pointed at, and the devtools connection.
//
// The one rule this file exists to keep: A LEAKED HOST IS THE WORST FAILURE
// MODE. It survives the test run, keeps a window on the user's screen, holds a
// lock on the executable, and answers on a port that a later run may believe is
// its own. So the kill happens from three places — `close()`, a `process.exit`
// hook, and the signal handlers — and all three are idempotent.

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, openSync, closeSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { Locator, type LocatorOptions } from "./locator";
import { Wire, readHandshake, pingOnce, VnCommandError } from "./wire";
import {
  allocateSlot,
  cacheDistFor,
  ensureBuild,
  ensureWorkDir,
  releaseStagedExe,
  resolveEntry,
  stagedExe,
  REPO,
  WORK_DIR,
} from "./dist";
import { artifactPath } from "./artifacts";

export interface LaunchOptions {
  /**
   * The app under test: a name under `apps/` (`"hello"`, `"shadcn"`) or a path
   * to an entry module relative to the workspace root.
   */
  app: string;
  /** `[width, height]` in CSS px — becomes `VN_WINDOW`. Default: the app's own `vn.config.json`. */
  window?: [number, number];
  /** Extra environment for the host process. `VN_DIST` is always ours. */
  env?: Record<string, string>;
  /** Default timeout for auto-waiting locators and assertions (ms). Default 10 000. */
  timeout?: number;
  /** How long to wait for the host to boot and mount (ms). Default 60 000. */
  launchTimeout?: number;
  /** Skip the build and use whatever is in the cache dist. Default false. */
  noBuild?: boolean;
  /** Wait for JS to have mounted a tree before returning. Default true. */
  waitForMount?: boolean;
  /** Print build/launch progress to stdout. Default false. */
  verbose?: boolean;
  /** A name for slot directories and artifacts. Default: the app name. */
  label?: string;
  /** Use this executable instead of a staged copy of `target/.../vn-app.exe`. */
  exe?: string;
  /**
   * `exe` is a PACKAGED app (`vn build`) whose `vn.config.json` name is this.
   * Nothing is built and no `VN_DIST` is set: the exe runs exactly as a user's
   * double-click would, and the harness finds it where a packaged app publishes
   * its handshake — `%TEMP%\vue-native\apps\<name>\<pid>` (docs/PACKAGING.md).
   */
  packaged?: string;
  /** Leave the process running at teardown — for debugging a red test by hand. */
  keepAlive?: boolean;
}

export type LogEntry = { seq: number; at: number; kind: string; text: string };
export type Bounds = { x: number; y: number; w: number; h: number };
export type NodeInfo = {
  id: number;
  tag?: string;
  classes?: string[];
  text?: string;
  bounds: Bounds | null;
  scopes?: string[];
  groupAnchor?: boolean;
  gone?: boolean;
};

const live = new Set<VnApp>();
let hooked = false;

/** Kills every host this process started. Must stay synchronous — see `hookExit`. */
function killAllSync() {
  for (const app of [...live]) app.killSync();
  releaseStagedExe();
}

function hookExit() {
  if (hooked) return;
  hooked = true;
  // `exit` is the belt: it runs after bun:test has finished, pass or fail, and
  // after an uncaught throw. It has to be synchronous, which is why the kill
  // path is `spawnSync` all the way down.
  process.on("exit", killAllSync);
  // …and the braces, for the Ctrl-C that never reaches `exit`.
  for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"] as const) {
    process.on(sig, () => {
      killAllSync();
      process.exit(130);
    });
  }
}

/**
 * Where this harness last put the pointer: a node (resolved fresh every time,
 * because a re-render moves boxes) or a window coordinate.
 */
export type PointerIntent =
  | { kind: "node"; selector: string; index: number }
  | { kind: "point"; x: number; y: number };

export class VnApp {
  readonly locatorDefaults: LocatorOptions;
  #closed = false;
  /**
   * The pointer, as a FACT THIS HARNESS OWNS rather than one it reads.
   *
   * gpui decides `:hover` from its own `mouse_position`, and that is a single
   * per-window value fed by whatever `PlatformInput` arrived last — the
   * synthetic move our `mouse` command delivers, *and every real one the
   * person at the machine makes*. So a spec that hovered and then asserted was
   * asserting about the LIVE USER'S POINTER from the moment they moved it:
   * `hover.spec.ts` passed alone and failed inside a full run, on a different
   * test each time, because a full run is long enough for a hand to move.
   *
   * The fix is not to retry harder — a retry re-reads the same overwritten
   * value. It is for the harness to place the pointer itself and to RE-PLACE
   * it immediately before every read that depends on it (see
   * [`settlePointer`]), so the last word on where the pointer is always
   * belongs to the test.
   */
  #pointer: PointerIntent | null = null;

  constructor(
    readonly wire: Wire,
    readonly child: ChildProcess,
    readonly pid: number,
    /** The ephemeral port this host bound. Different for every instance. */
    readonly port: number,
    readonly slotDir: string,
    readonly label: string,
    readonly hostLog: string,
    readonly timeout: number,
    private readonly keepAlive: boolean,
    private readonly logFd: number,
  ) {
    this.locatorDefaults = { timeout };
  }

  // -- selection ------------------------------------------------------------

  /**
   * A handle on whatever `selector` matches, resolved fresh on every use.
   *
   * Nothing is queried here — like Playwright's, a locator is a description,
   * not a snapshot, so it survives the tree being rebuilt under it.
   */
  locator(selector: string, options: LocatorOptions = {}): Locator {
    return new Locator(this, selector, { ...this.locatorDefaults, ...options });
  }

  // -- the pointer ----------------------------------------------------------

  /**
   * Records where a gesture just put the pointer. Called by `locator.hover()`,
   * `locator.unhover()` and a real `locator.click()` — every command that
   * delivers a `PlatformInput` move.
   */
  notePointer(intent: PointerIntent): void {
    this.#pointer = intent;
  }

  /**
   * Puts the pointer back where this harness last put it, and waits for the
   * host to have processed the move.
   *
   * Called before every hit-test-dependent read (`locator.styles()`, which
   * carries both `hovered` and the `live` style bucket). It is a no-op until
   * something has placed the pointer, so a spec that never hovers never pays
   * for it and never has a stray move delivered into it.
   *
   * `force` is for an assertion whose SUBJECT is the pointer
   * (`toBeHovered`, and its `.not`): a claim that the pointer is or is not
   * over an element is only meaningful if the harness knows where it is, so
   * those settle it to "away" (1, 1) rather than reading whatever the person
   * at the machine left behind.
   */
  async settlePointer(force = false): Promise<void> {
    if (this.#closed) return;
    let intent = this.#pointer;
    if (!intent) {
      if (!force) return;
      intent = { kind: "point", x: 1, y: 1 };
      this.#pointer = intent;
    }
    const args =
      intent.kind === "node"
        ? { selector: intent.selector, index: intent.index, kind: "move" }
        : { x: intent.x, y: intent.y, kind: "move" };
    // A failure here is not the assertion's failure: the node may have gone
    // away between the hover and the read, and the matcher's own retry is the
    // thing that should report that.
    await this.send("mouse", args).catch(() => {});
  }

  // -- raw commands ---------------------------------------------------------

  /** One devtools command, unwrapped to its `data`. The escape hatch under everything. */
  send(cmd: string, args: Record<string, unknown> = {}, timeoutMs?: number): Promise<any> {
    return this.wire.send(cmd, args, timeoutMs);
  }

  query(selector: string): Promise<{ count: number; nodes: NodeInfo[]; warnings?: string[] }> {
    return this.send("query", { selector });
  }

  tree(selector?: string, depth?: number): Promise<any> {
    return this.send("tree", { ...(selector ? { selector } : {}), ...(depth ? { depth } : {}) });
  }

  /**
   * Evaluates `js` in the app's JSC, in global scope.
   *
   * A THROW inside the app is a successful round trip carrying a failure — the
   * host reports it as `{ok:true, data:{exception}}`, exactly like a devtools
   * console. We turn it back into a thrown Error, because a test that says
   * `await app.eval(...)` means "do this", not "tell me how it went".
   */
  async eval(js: string): Promise<any> {
    const data = await this.send("eval", { js });
    if (data?.exception) {
      const e = new Error(`app.eval threw: ${data.exception}`);
      e.stack = `${e.message}\n${data.stack ?? ""}`;
      throw e;
    }
    return data?.value;
  }

  /**
   * The host's log ring since `since` (a cursor from a previous call).
   * 500 entries deep; `next` is the cursor to pass next time.
   */
  logs(since = 0): Promise<{ entries: LogEntry[]; next: number; status: string | null; dropped?: number }> {
    return this.send("logs", { since });
  }

  /** Client-area readback (never raises the window). Returns the path written. */
  async screenshot(path?: string, backend?: "auto" | "render_to_image" | "print_window"): Promise<string> {
    const target = path ? resolve(path) : artifactPath(`${this.label}-screenshot`, "png");
    mkdirSync(dirname(target), { recursive: true });
    const data = await this.send("screenshot", { path: target, ...(backend ? { backend } : {}) });
    return data.path as string;
  }

  /** A real keystroke to whatever currently has focus. `"Enter"`, `"Shift+Tab"`, `"ArrowDown"`. */
  press(key: string): Promise<any> {
    return this.send("key", { key });
  }

  // -- diagnostics ----------------------------------------------------------

  /**
   * Fails if the app logged anything that is genuinely broken.
   *
   * `[js uncaught]` is always a failure. CSS advisories are NOT, by default:
   * they are the compat-first design working as intended (an unsupported
   * declaration says so instead of silently doing nothing), and a spec that
   * treated every advisory as a failure would go red the day someone adds a
   * `backdrop-filter`. Opt in with `{ advisories: true }` when the point of the
   * test IS the advisory stream.
   */
  async expectNoErrors(options: { advisories?: boolean; ignore?: RegExp[] } = {}) {
    const { entries } = await this.logs(0);
    const ignore = options.ignore ?? [];
    const bad = entries.filter((e) => {
      if (ignore.some((re) => re.test(e.text))) return false;
      if (e.kind === "error") return true;
      if (e.text.includes("[js uncaught]")) return true;
      if (options.advisories && (e.kind === "advisory" || e.kind === "css")) return true;
      return false;
    });
    if (!bad.length) return;
    const shot = await this.screenshot(artifactPath(`${this.label}-errors`, "png")).catch(() => "(no screenshot)");
    const listing = bad.map((e) => `  [${e.kind}] ${e.text}`).join("\n");
    throw new Error(
      `${this.label}: the app logged ${bad.length} error(s)\n${listing}\n` +
        `  screenshot: ${shot}\n  host log:   ${this.hostLog}`,
    );
  }

  /** The tail of the log ring, for a failure message. Never throws. */
  async tailLogs(n = 12): Promise<string> {
    try {
      const { entries } = await this.logs(0);
      return entries
        .slice(-n)
        .map((e) => `  [${e.kind}] ${e.text}`)
        .join("\n");
    } catch (e) {
      return `  (logs unavailable: ${(e as Error).message})`;
    }
  }

  // -- lifecycle ------------------------------------------------------------

  get closed() {
    return this.#closed;
  }

  /** Kill the host and drop the connection. Idempotent; safe in `afterAll`. */
  async close() {
    if (this.#closed) return;
    this.killSync();
  }

  /**
   * The synchronous kill. `/T` takes the tree (a host that spawned helpers),
   * `/F` skips the polite close a GUI process can decline. Called from
   * `process.on("exit")`, where nothing asynchronous can run.
   */
  killSync() {
    if (this.#closed) return;
    this.#closed = true;
    live.delete(this);
    try {
      this.wire.close();
    } catch {}
    if (!this.keepAlive) {
      try {
        spawnSync("taskkill", ["/PID", String(this.pid), "/T", "/F"], { stdio: "ignore" });
      } catch {}
    }
    try {
      closeSync(this.logFd);
    } catch {}
    if (!this.keepAlive) {
      // The slot is a copy; nothing in it is source. Leaving it behind would
      // grow without bound across runs.
      try {
        rmSync(this.slotDir, { recursive: true, force: true });
      } catch {
        // Windows can still hold the exe's view of the dist for a beat.
      }
    }
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/**
 * `%TEMP%\vue-native\apps\<name>\<pid>` — where a packaged app publishes its
 * handshake. The name is cleaned with the host's own rule
 * (`payload::slot_name`, crates/vn-app/src/payload.rs), so the two agree.
 */
export function packagedSlot(name: string, pid: number): string {
  const cleaned = [...name]
    .map((c) => (/[A-Za-z0-9._-]/.test(c) ? c : "-"))
    .slice(0, 64)
    .join("")
    .replace(/^[-.]+|[-.]+$/g, "");
  return join(tmpdir(), "vue-native", "apps", cleaned || "vn-host", String(pid));
}

/**
 * The oldest bun this harness is known to survive, and why there is a floor at
 * all.
 *
 * `killSync()` calls `spawnSync("taskkill", …)` — from `close()`, from a
 * `process.exit` hook and from the signal handlers, because A LEAKED HOST IS
 * THE WORST FAILURE MODE. On **bun 1.2.8** that `spawnSync` intermittently
 * tripped `STATUS_HEAP_CORRUPTION` inside bun's own process spawning, which
 * aborts the whole runner with exit code 116 — no failing test, no stack, just
 * a dead process. It reproduced on whole-file runs of `devtools-v2.spec.ts`,
 * which closes and relaunches more hosts than any other spec.
 *
 * bun **1.4.0** does not reproduce it (three consecutive whole-suite runs, 86
 * tests across 10 files, zero aborts — research/reports/m22/adoptions-1.md).
 * The floor is stated rather than assumed because the failure mode is a
 * process abort: a suite running on an older bun does not fail informatively,
 * it vanishes, and the next person to see exit 116 should not have to
 * rediscover why.
 */
export const MIN_BUN = "1.4.0";

/** `1.4.0` -> `[1, 4, 0]`, ignoring any `-canary.…` suffix. */
function semver(v: string): number[] {
  return v
    .split("-")[0]!
    .split(".")
    .map((n) => Number.parseInt(n, 10) || 0);
}

function olderThan(have: string, want: string): boolean {
  const a = semver(have);
  const b = semver(want);
  for (let i = 0; i < 3; i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) < (b[i] ?? 0);
  }
  return false;
}

let bunChecked = false;

/** Fails the run before the first host starts, with the reason spelled out. */
function requireBun() {
  if (bunChecked) return;
  bunChecked = true;
  const have = typeof Bun !== "undefined" ? Bun.version : "";
  if (!have) {
    throw new Error(
      "vn-test runs on bun (`bun test packages/vn-test/specs`) — this process is not bun.",
    );
  }
  if (olderThan(have, MIN_BUN)) {
    throw new Error(
      `vn-test needs bun >= ${MIN_BUN}; this is ${have}.\n` +
        "  Older bun aborts the whole runner with STATUS_HEAP_CORRUPTION (exit 116) inside the\n" +
        "  `spawnSync(\"taskkill\")` that VnApp.killSync uses to make sure no host survives the\n" +
        "  run. The abort takes the process, not a test, so a run on an old bun looks like a\n" +
        "  crash rather than a failure.\n" +
        "  Upgrade with `bun upgrade` (or `npm i -g bun`), then re-run. See docs/TESTING.md.",
    );
  }
}

/**
 * Builds if stale, allocates a private slot, starts a host on it, and waits
 * until it answers.
 *
 * NOTE: no `windowsHide`. It sets `STARTF_USESHOWWINDOW`/`SW_HIDE`, which for a
 * GUI process hides the MAIN WINDOW — and gpui stops producing frames for a
 * hidden window, so the devtools listener would accept the connection and then
 * never answer. There is no headless mode; the window opens. Nothing here
 * focuses it, moves a cursor, or sends global input: every gesture in this
 * package is a devtools command.
 */
export async function launch(options: LaunchOptions): Promise<VnApp> {
  requireBun();
  ensureWorkDir();
  hookExit();

  const log = options.verbose ? (l: string) => console.log(l) : () => {};
  const label = options.label ?? options.app.replace(/[\\/]/g, "-");
  const timeout = options.timeout ?? 10_000;
  const launchTimeout = options.launchTimeout ?? 60_000;

  if (options.packaged !== undefined && !options.exe) {
    throw new Error(`${label}: \`packaged\` needs \`exe\` — the packaged app to run`);
  }
  // A packaged app's slot is per process, so it is only known once the pid
  // is; a dev host's is allocated (and its handshake deleted) up front.
  let slotDir = "";
  if (options.packaged === undefined) {
    const entry = resolveEntry(options.app);
    const cache = options.noBuild ? cacheDistFor(entry) : await ensureBuild(entry, log);
    if (!existsSync(join(cache, "manifest.json"))) {
      throw new Error(`${cache} has no manifest.json — drop \`noBuild\` or run the builder first`);
    }
    slotDir = allocateSlot(cache, label);
  }
  const exe = options.exe ? resolve(options.exe) : stagedExe();

  const logsDir = join(WORK_DIR, "logs");
  mkdirSync(logsDir, { recursive: true });
  const hostLog = join(logsDir, `${label}-${process.pid}-${Date.now()}.log`);
  const logFd = openSync(hostLog, "w");

  const env: Record<string, string> = { ...(process.env as Record<string, string>), VN_DIST: slotDir };
  if (options.packaged !== undefined) {
    // Exactly the user's environment: no slot of ours, no dev-only pointers.
    for (const name of ["VN_DIST", "VN_APP_ROOT", "VN_PAYLOAD", "VN_APP_NAME"]) delete env[name];
  }
  if (options.window) env.VN_WINDOW = `${options.window[0]}x${options.window[1]}`;
  // The host holds a handle on THIS process and terminates itself when it
  // exits — the one kill path that survives the runner being killed
  // (crates/vn-app/src/owner.rs). Not for `keepAlive`, whose whole point is
  // to outlive the run.
  if (options.keepAlive) delete env.VN_OWNER_PID;
  else env.VN_OWNER_PID = String(process.pid);
  Object.assign(env, options.env ?? {});

  log(`[vn-test] ${label}: launching ${exe} on ${slotDir || "its own %TEMP% slot"}`);
  const child = spawn(exe, [], {
    cwd: REPO,
    detached: true,
    stdio: ["ignore", logFd, logFd],
    env,
  });
  child.unref();
  const pid = child.pid;
  if (!pid) {
    closeSync(logFd);
    throw new Error(`could not start ${exe}`);
  }

  let exited: number | null = null;
  child.on("exit", (code) => {
    exited = code ?? -1;
  });

  if (options.packaged !== undefined) slotDir = packagedSlot(options.packaged, pid);
  const handshakePath = join(slotDir, "devtools.json");
  const deadline = Date.now() + launchTimeout;
  const fail = (why: string) => {
    try {
      spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
      closeSync(logFd);
    } catch {}
    return new Error(
      `${label}: ${why}\n  exe:      ${exe}\n  dist:     ${slotDir}\n  host log: ${hostLog}`,
    );
  };

  // Phase 1: the handshake file. It was deleted from the slot before the spawn
  // (see `allocateSlot`), so its reappearance is a real edge, not a guess.
  while (!existsSync(handshakePath)) {
    if (exited !== null) throw fail(`the host exited with code ${exited} before writing devtools.json`);
    if (Date.now() > deadline) throw fail(`no devtools.json after ${launchTimeout} ms`);
    await sleep(25);
  }

  // Phase 2: a reply. The listener starts before the JS runtime, so this only
  // proves the process is alive and pumping frames.
  const handshake = readHandshake(slotDir);
  let ping: any = null;
  for (;;) {
    try {
      ping = await pingOnce(handshake.port, handshake.token, 5_000);
      break;
    } catch (e) {
      if (exited !== null) throw fail(`the host exited with code ${exited} during startup`);
      if (Date.now() > deadline) throw fail(`ping never answered: ${(e as Error).message}`);
      await sleep(50);
    }
  }

  const wire = new Wire(handshake, Math.max(timeout, 15_000));
  const app = new VnApp(
    wire,
    child,
    pid,
    handshake.port,
    slotDir,
    label,
    hostLog,
    timeout,
    options.keepAlive ?? false,
    logFd,
  );
  live.add(app);

  // Phase 3: a MOUNTED tree. `ping` answers `{root: 0}` from the moment the
  // listener is up, which is well before vendor.js has been evaluated — a
  // locator resolved in that window would report "matched no nodes" and look
  // like a selector bug. Waiting here is what makes the first assertion in a
  // spec mean what it says.
  if (options.waitForMount !== false) {
    for (;;) {
      if (ping?.root) break;
      if (exited !== null) throw fail(`the host exited with code ${exited} before mounting`);
      if (Date.now() > deadline) {
        const tail = await app.tailLogs(20);
        app.killSync();
        throw new Error(
          `${label}: nothing mounted within ${launchTimeout} ms (ping still reports root 0)\n` +
            `${tail}\n  host log: ${hostLog}`,
        );
      }
      await sleep(50);
      ping = await app.send("ping").catch(() => null);
    }
  }

  log(`[vn-test] ${label}: ready (pid ${pid}, root ${ping?.root})`);
  return app;
}

export { VnCommandError };
