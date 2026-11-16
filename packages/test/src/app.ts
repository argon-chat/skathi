// So the kill happens from three places — `close()`, a `process.exit` hook, and the signal
// handlers — and all three are idempotent.

import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, openSync, closeSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { Locator, type LocatorOptions } from "./locator";
import { Wire, readHandshake, pingOnce, SkCommandError } from "./wire";
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
import { killTree } from "./reap";

export interface LaunchOptions {
  /**
   * The app under test: a name under `apps/` (`"hello"`, `"shadcn"`) or a path to an entry module
   * relative to the workspace root.
   */
  app: string;
  /** `[width, height]` in CSS px — becomes `SK_WINDOW`. */
  window?: [number, number];
  /** Extra environment for the host process. */
  env?: Record<string, string>;
  /** Default timeout for auto-waiting locators and assertions (ms). */
  timeout?: number;
  /** How long to wait for the host to boot and mount (ms). */
  launchTimeout?: number;
  /** Skip the build and use whatever is in the cache dist. */
  noBuild?: boolean;
  /** Wait for JS to have mounted a tree before returning. */
  waitForMount?: boolean;
  /** Print build/launch progress to stdout. */
  verbose?: boolean;
  /** A name for slot directories and artifacts. */
  label?: string;
  /** Use this executable instead of a staged copy of `target/.../sk-app.exe`. */
  exe?: string;
  /** `exe` is a PACKAGED app (`sk build`) whose `sk.config.json` name is this. */
  packaged?: string;
  /** Leave the process running at teardown — for debugging a red test by hand. */
  keepAlive?: boolean;
  args?: string[];
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
export type PendingDialog = {
  id: number;
  /** The owner window's key. */
  window: number;
  what: "picker" | "download";
  kind: "open" | "folder" | "save";
  multiple: boolean;
  filters: { name: string; spec: string }[];
  allowAll: boolean;
  startIn: string | null;
  suggestedName: string | null;
  [more: string]: unknown;
};
/** A question waiting for a button: the app's permission prompt (`dialog list`, kind "ask"). */
export type PendingQuestion = {
  id: number;
  window: number;
  what: "permission";
  kind: "ask";
  title: string;
  instruction: string;
  content: string;
  /** Labels, access-key `&` removed: "Allow", "Block". */
  buttons: string[];
  default: string | null;
};
/** `permissions list` (crates/sk-permissions `describe_json`). */
export type PermissionsInfo = {
  states: { name: string; state: "granted" | "denied" | "prompt" | "unsupported"; prompts: boolean }[];
  decisions: { name: string; decision: "allow" | "block"; at: number }[];
  file: string | null;
};

const live = new Set<SkApp>();
let hooked = false;

/** Kills every host this process started. */
function killAllSync() {
  for (const app of [...live]) app.killSync();
  releaseStagedExe();
}

function hookExit() {
  if (hooked) return;
  hooked = true;
  // It has to be synchronous, which is why the kill path is `spawnSync` all the way down.
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
 * Where this harness last put the pointer: a node (resolved fresh every time, because a re-render
 * moves boxes) or a window coordinate.
 */
export type PointerIntent = { kind: "node"; selector: string; index: number } | { kind: "point"; x: number; y: number };

export class SkApp {
  readonly locatorDefaults: LocatorOptions;
  #closed = false;
  #pointer: PointerIntent | null = null;

  constructor(
    readonly wire: Wire,
    readonly child: ChildProcess,
    readonly pid: number,
    /** The ephemeral port this host bound. */
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

  /** A handle on whatever `selector` matches, resolved fresh on every use. */
  locator(selector: string, options: LocatorOptions = {}): Locator {
    return new Locator(this, selector, { ...this.locatorDefaults, ...options });
  }

  /** Records where a gesture just put the pointer. */
  notePointer(intent: PointerIntent): void {
    this.#pointer = intent;
  }

  /**
   * Puts the pointer back where this harness last put it, and waits for the host to have processed
   * the move.
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
    await this.send("mouse", args).catch(() => {});
  }

  /** One devtools command, unwrapped to its `data`. */
  send(cmd: string, args: Record<string, unknown> = {}, timeoutMs?: number): Promise<any> {
    return this.wire.send(cmd, args, timeoutMs);
  }

  query(selector: string): Promise<{ count: number; nodes: NodeInfo[]; warnings?: string[] }> {
    return this.send("query", { selector });
  }

  tree(selector?: string, depth?: number): Promise<any> {
    return this.send("tree", { ...(selector ? { selector } : {}), ...(depth ? { depth } : {}) });
  }

  /** Evaluates `js` in the app's JSC, in global scope. */
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
   * The host's log ring since `since` (a cursor from a previous call). 500 entries deep; `next` is
   * the cursor to pass next time.
   */
  logs(since = 0): Promise<{ entries: LogEntry[]; next: number; status: string | null; dropped?: number }> {
    return this.send("logs", { since });
  }

  /** Client-area readback (never raises the window). */
  async screenshot(path?: string, backend?: "auto" | "render_to_image" | "print_window"): Promise<string> {
    const target = path ? resolve(path) : artifactPath(`${this.label}-screenshot`, "png");
    mkdirSync(dirname(target), { recursive: true });
    const data = await this.send("screenshot", { path: target, ...(backend ? { backend } : {}) });
    return data.path as string;
  }

  /** A real keystroke to whatever currently has focus. */
  press(key: string): Promise<any> {
    return this.send("key", { key });
  }

  /** The file dialogs waiting for an answer, oldest first (questions are `questions()`). */
  async dialogs(): Promise<PendingDialog[]> {
    const all: (PendingDialog | PendingQuestion)[] = await this.send("dialog", { op: "list" });
    return all.filter((d): d is PendingDialog => d.kind !== "ask");
  }

  /** The questions waiting for a button, oldest first: the permission prompt. */
  async questions(): Promise<PendingQuestion[]> {
    const all: (PendingDialog | PendingQuestion)[] = await this.send("dialog", { op: "list" });
    return all.filter((d): d is PendingQuestion => d.kind === "ask");
  }

  /** Waits until a question is pending and returns the oldest one. */
  async waitForQuestion(timeoutMs = 5000): Promise<PendingQuestion> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const [first] = await this.questions();
      if (first) return first;
      if (Date.now() > deadline) throw new Error(`no question (permission prompt) opened within ${timeoutMs} ms`);
      await new Promise((r) => setTimeout(r, 25));
    }
  }

  /** Presses `button` (its label, `"Allow"` / `"Block"`, or its index) in question `id`. */
  chooseDialog(id: number, button: string | number): Promise<{ id: number; chose: string }> {
    return this.send("dialog", { op: "choose", id, button: String(button) });
  }

  /** The app's permission states, its stored prompt answers and their file (`permissions`). */
  permissions(): Promise<PermissionsInfo> {
    return this.send("permissions", { op: "list" });
  }

  /**
   * Forgets the stored prompt answer for `name` (every one without), so the next request asks
   * again.
   */
  resetPermissions(name?: string): Promise<{ reset: string[]; now: PermissionsInfo }> {
    return this.send("permissions", name ? { op: "reset", name } : { op: "reset" });
  }

  /**
   * Waits until a file dialog is pending and returns the oldest one: the dialog thread parks it a
   * moment after the click that opened it.
   */
  async waitForDialog(timeoutMs = 5000): Promise<PendingDialog> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const [first] = await this.dialogs();
      if (first) return first;
      if (Date.now() > deadline) throw new Error(`no file dialog opened within ${timeoutMs} ms`);
      await new Promise((r) => setTimeout(r, 25));
    }
  }

  /** Finishes dialog `id` with absolute paths, as the user picking them would. */
  answerDialog(id: number, paths: string | string[]): Promise<{ id: number; answered: number }> {
    return this.send("dialog", { op: "answer", id, paths: Array.isArray(paths) ? paths : [paths] });
  }

  /** Dismisses dialog `id`, as Esc would. */
  cancelDialog(id: number): Promise<{ id: number; cancelled: true }> {
    return this.send("dialog", { op: "cancel", id });
  }

  /** Fails if the app logged anything that is genuinely broken. */
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

  /** The tail of the log ring, for a failure message. */
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

  get closed() {
    return this.#closed;
  }

  /** Kill the host and drop the connection. */
  async close() {
    if (this.#closed) return;
    this.killSync();
  }

  /** The synchronous kill. */
  killSync() {
    if (this.#closed) return;
    this.#closed = true;
    live.delete(this);
    try {
      this.wire.close();
    } catch {}
    if (!this.keepAlive) {
      try {
        killTree(this.pid);
      } catch {}
    }
    try {
      closeSync(this.logFd);
    } catch {}
    if (!this.keepAlive) {
      // Leaving it behind would grow without bound across runs.
      try {
        rmSync(this.slotDir, { recursive: true, force: true });
      } catch {
        // Windows can still hold the exe's view of the dist for a beat.
      }
    }
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** `%TEMP%\skathi\apps\<name>\<pid>` — where a packaged app publishes its handshake. */
export function packagedSlot(name: string, pid: number): string {
  const cleaned = [...name]
    .map((c) => (/[A-Za-z0-9._-]/.test(c) ? c : "-"))
    .slice(0, 64)
    .join("")
    .replace(/^[-.]+|[-.]+$/g, "");
  return join(tmpdir(), "skathi", "apps", cleaned || "sk-host", String(pid));
}

/** The oldest bun this harness is known to survive, and why there is a floor at all. */
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
    throw new Error("sk-test runs on bun (`bun test packages/test/specs`) — this process is not bun.");
  }
  if (olderThan(have, MIN_BUN)) {
    throw new Error(
      `sk-test needs bun >= ${MIN_BUN}; this is ${have}.\n` +
        "  Older bun aborts the whole runner with STATUS_HEAP_CORRUPTION (exit 116) inside the\n" +
        '  `spawnSync("taskkill")` that SkApp.killSync uses to make sure no host survives the\n' +
        "  run. The abort takes the process, not a test, so a run on an old bun looks like a\n" +
        "  crash rather than a failure.\n" +
        "  Upgrade with `bun upgrade` (or `npm i -g bun`), then re-run.",
    );
  }
}

/** Builds if stale, allocates a private slot, starts a host on it, and waits until it answers. */
export async function launch(options: LaunchOptions): Promise<SkApp> {
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
  // A packaged app's slot is per process, so it is only known once the pid is; a dev host's is
  // allocated (and its handshake deleted) up front.
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

  const env: Record<string, string> = { ...(process.env as Record<string, string>), SK_DIST: slotDir };
  if (options.packaged !== undefined) {
    for (const name of ["SK_DIST", "SK_APP_ROOT", "SK_PAYLOAD", "SK_APP_NAME"]) delete env[name];
  }
  if (options.window) env.SK_WINDOW = `${options.window[0]}x${options.window[1]}`;
  if (options.keepAlive) delete env.SK_OWNER_PID;
  else env.SK_OWNER_PID = String(process.pid);
  // A dev host's instance key includes its slot, so parallel slots never collide anyway — but a
  // spec that relaunches one slot must get a second host, not a forwarded launch.
  if (env.SK_SINGLE_INSTANCE === undefined) env.SK_SINGLE_INSTANCE = "0";
  if (env.SK_LAUNCH_RAISE === undefined) env.SK_LAUNCH_RAISE = "off";
  // A real dialog would block the run on a window nobody is there to answer, and steal the
  // foreground from whoever is.
  if (env.SK_DIALOGS === undefined) env.SK_DIALOGS = "scripted";
  // A test window opens BEHIND the person's windows and never takes the foreground: no activation
  // at open, no `window.focus()` raise or taskbar flash (crates/sk-app/src/quiet.rs).
  if (env.SK_WINDOW_FOCUS === undefined) env.SK_WINDOW_FOCUS = "never";
  Object.assign(env, options.env ?? {});
  if (env.SK_MEDIA_CAPTURE?.trim().toLowerCase() !== "fake") env.SK_MEDIA_CAPTURE = "deny";
  // Never the real cursor: the person at the machine keeps it. `fake` simulates the lock in gpui.
  if (env.SK_POINTER_LOCK?.trim().toLowerCase() !== "fake") env.SK_POINTER_LOCK = "deny";

  log(`[sk-test] ${label}: launching ${exe} on ${slotDir || "its own %TEMP% slot"}`);
  const child = spawn(exe, options.args ?? [], {
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
      killTree(pid);
      closeSync(logFd);
    } catch {}
    return new Error(`${label}: ${why}\n  exe:      ${exe}\n  dist:     ${slotDir}\n  host log: ${hostLog}`);
  };

  // It was deleted from the slot before the spawn (see `allocateSlot`), so its reappearance is a
  // real edge, not a guess.
  while (!existsSync(handshakePath)) {
    if (exited !== null) throw fail(`the host exited with code ${exited} before writing devtools.json`);
    if (Date.now() > deadline) throw fail(`no devtools.json after ${launchTimeout} ms`);
    await sleep(25);
  }

  // The listener starts before the JS runtime, so this only proves the process is alive and
  // pumping frames.
  // The host may not have finished writing it.
  let handshake: ReturnType<typeof readHandshake> | null = null;
  while (!handshake) {
    try {
      handshake = readHandshake(slotDir);
    } catch (e) {
      if (Date.now() > deadline) throw fail(`devtools.json is unreadable: ${(e as Error).message}`);
      await sleep(25);
    }
  }
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
  const app = new SkApp(
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

  log(`[sk-test] ${label}: ready (pid ${pid}, root ${ping?.root})`);
  return app;
}

export { SkCommandError };
