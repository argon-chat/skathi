import { markRaw, reactive } from "vue";
import { Push, devtoolsArgs, listTargets, message, send } from "./wire";
import { nodeLabel, rowVisible } from "./dump";
import { gcSummary, memUnsupported, sampleOf, type MemSample } from "./memfmt";
import type {
  ConsoleLine,
  EvalReply,
  LogsReply,
  MenuItem,
  NetDetail,
  NetEntry,
  NetReply,
  NetTab,
  NodeId,
  PoolStats,
  MemBreakdown,
  MemReply,
  MemUnsupported,
  PerfReply,
  PickReply,
  PingReply,
  PushMessage,
  PushStream,
  Row,
  RulesReply,
  ScreenshotReply,
  Shot,
  Status,
  StorageReply,
  StylesReply,
  Tab,
  Target,
  TargetList,
  TreeNode,
  WindowId,
  WindowInfo,
  WindowsReply,
} from "./protocol";

const SELECT_TTL_MS = 6000;
const LOG_LIMIT = 1000;
const NET_LIMIT = 400;
const PERF_HISTORY = 240;
const MEM_HISTORY = 120;

export const LEVELS = ["error", "warn", "info", "debug"] as const;
export type Level = (typeof LEVELS)[number];

export function levelOf(kind: string): Level {
  switch (kind) {
    case "error":
      return "error";
    case "warn":
    case "css":
    case "advisory":
    case "window":
      return "warn";
    case "dev":
    case "js":
      return "debug";
    default:
      return "info";
  }
}

interface State {
  session: number;
  captureActive: boolean;
  targets: Target[];
  target: Target | null;
  status: Status;
  error: string;
  pushLive: boolean;
  pushNote: string;

  // -- windows of the target
  windows: WindowInfo[];
  window: WindowId | null;
  inspectors: { window: WindowId | null; pid: number }[];

  tab: Tab;
  consoleOpen: boolean;
  treeWidth: number;
  consoleHeight: number;
  dragging: "tree" | "console" | "net" | null;

  tree: TreeNode | null;
  rows: Row[];
  byId: Record<NodeId, TreeNode>;
  expanded: Record<NodeId, boolean>;
  selected: NodeId | null;
  treeError: string;
  treeRev: number;
  seenRev: number;
  elementSearch: string;
  matches: NodeId[];
  matchIndex: number;
  treeScroll: number;
  showAnchors: boolean;

  styles: StylesReply | null;
  rules: RulesReply | null;
  stylesTab: string;
  stylesError: string;
  copied: string;

  logs: ConsoleLine[];
  logCursor: number;
  logStatus: string;
  dropped: number;
  logFilter: string;
  levels: Record<Level, boolean>;
  preserve: boolean;
  consoleScroll: number;
  follow: boolean;
  evalHistory: string[];
  evalIndex: number;

  picking: boolean;
  pickInfo: PickReply | null;

  perf: PerfReply | null;
  perfHistory: number[];
  mem: MemBreakdown | null;
  memHistory: MemSample[];
  memError: string;
  /** The host's reason; "" otherwise. */
  memUnsupported: string;
  memCollectMs: number;
  memGcBusy: boolean;
  memResident: { at: number; breakdown: MemBreakdown } | null;
  memResidentBusy: boolean;

  net: NetEntry[];
  netCursor: number;
  netFilter: string;
  netSelected: number | null;
  netDetail: NetDetail | null;
  netDetailError: string;
  netDetailBusy: boolean;
  netTab: NetTab;
  netListWidth: number;
  netPool: PoolStats | null;

  storageArea: string;
  storage: { key: string; value: string }[];
  storageError: string;

  shot: Shot | null;
  shotError: string;
  shotBusy: boolean;
}

export const state = reactive<State>({
  session: 0,
  captureActive: false,
  targets: [],
  target: null,
  status: "detached",
  error: "",
  pushLive: false,
  pushNote: "",

  windows: [],
  window: null,
  inspectors: [],

  tab: "elements",
  consoleOpen: true,
  treeWidth: 460,
  consoleHeight: 240,
  dragging: null,

  tree: null,
  rows: [],
  byId: {},
  expanded: {},
  selected: null,
  treeError: "",
  treeRev: 0,
  seenRev: -1,
  elementSearch: "",
  matches: [],
  matchIndex: 0,
  treeScroll: 0,
  showAnchors: false,

  styles: null,
  rules: null,
  stylesTab: "rules",
  stylesError: "",
  copied: "",

  logs: [],
  logCursor: 0,
  logStatus: "",
  dropped: 0,
  logFilter: "",
  levels: { error: true, warn: true, info: true, debug: true },
  preserve: false,
  consoleScroll: 0,
  follow: true,
  evalHistory: [],
  evalIndex: -1,

  picking: false,
  pickInfo: null,

  perf: null,
  perfHistory: [],
  mem: null,
  memHistory: [],
  memError: "",
  memUnsupported: "",
  memCollectMs: 0,
  memGcBusy: false,
  memResident: null,
  memResidentBusy: false,

  net: [],
  netCursor: 0,
  netFilter: "",
  netSelected: null,
  netDetail: null,
  netDetailError: "",
  netDetailBusy: false,
  netTab: "headers",
  netListWidth: 420,
  netPool: null,

  storageArea: "local",
  storage: [],
  storageError: "",

  shot: null,
  shotError: "",
  shotBusy: false,
});

export const args = devtoolsArgs();

export const ui = reactive({
  typing: false,
  openElementSearch: null as null | (() => void),
  /** Escape closes it before it reaches the drawer. */
  elementSearchOpen: false,
  closeElementSearch: null as null | (() => void),
  focusPrompt: null as null | (() => void),
  /**
   * Two panels have a selection at once (the console drawer sits under Network), and "the one you
   * touched" is the only answer a person would expect.
   */
  copyTarget: "" as "" | "console" | "network",
  menu: null as null | { x: number; y: number; items: MenuItem[] },
  flash: "",
  flashBad: false,
});

function cmd<T>(name: string, cmdArgs: Record<string, unknown> = {}, timeoutMs?: number): Promise<T> {
  const target = state.target,
    session = state.session;
  const scoped = state.window === null ? cmdArgs : { window: state.window, ...cmdArgs };
  const current = () => state.session === session;
  return send<T>(target, name, scoped, timeoutMs, { isCurrent: current }).then((reply) => {
    if (!current()) throw new Error("attachment changed");
    return reply;
  });
}

/** A detached inspector must not leave instrumentation running in its old target. */
export function stopCapture(): void {
  if (!state.target || !state.captureActive) return;
  const target = state.target;
  state.captureActive = false;
  void send(target, "trace", { op: "recording-stop" }).catch(() => {});
  void send(target, "allocations", { op: "stop" }).catch(() => {});
}

export async function refreshTargets(): Promise<TargetList> {
  const found = await listTargets();
  state.targets = found.targets || [];
  state.error = found.error || "";
  return found;
}

export function bestTarget(found?: TargetList | null): Target | null {
  const live = state.targets.filter((t) => t.alive);
  const preferred = found?.preferred || args.target;
  if (preferred) {
    const named = live.find((t) => t.dir === preferred || t.slot === preferred);
    if (named) return named;
  }
  return live.find((t) => !t.self) || live[0] || null;
}

let channel: Push | null = null;

export async function attach(target: Target | null): Promise<void> {
  stopCapture();
  if (state.picking) void cancelPick();
  closePush();
  state.target = target;
  state.status = target ? "attaching" : "detached";
  state.error = "";
  resetInspection();
  if (!target) return;
  const session = state.session;
  try {
    const pong = await send<PingReply>(target, "ping");
    if (session !== state.session) return;
    state.status = "attached";
    state.logCursor = 0;
    note({ kind: "dev", text: `attached to ${target.slot} · pid ${pong.pid} · port ${target.port}` });
    await refreshWindows();
    if (session !== state.session) return;
    if (args.window !== null && state.windows.some((w) => w.id === args.window)) {
      state.window = args.window;
    }
    openPush();
    await refreshTree();
    await pollLogs();
    await refreshNet();
  } catch (e) {
    if (session !== state.session) return;
    state.status = "lost";
    state.error = message(e);
  }
}

export function detach(): void {
  stopCapture();
  if (state.picking) void cancelPick();
  if (state.target) clearHighlight();
  closePush();
  state.target = null;
  state.status = "detached";
  resetInspection();
}

function resetInspection(): void {
  inspectionEpoch++;
  state.session++;
  state.captureActive = false;
  cancelTreeRefresh();
  state.tree = null;
  state.rows = [];
  state.byId = {};
  state.expanded = {};
  state.selected = null;
  state.styles = null;
  state.rules = null;
  if (!state.preserve) state.logs = [];
  state.logCursor = 0;
  state.picking = false;
  state.pickInfo = null;
  state.shot = null;
  state.windows = [];
  state.window = null;
  state.perf = null;
  state.perfHistory = [];
  state.mem = null;
  state.memHistory = [];
  state.memError = "";
  state.memUnsupported = "";
  state.memResident = null;
  state.net = [];
  state.netCursor = 0;
  state.netSelected = null;
  state.netDetail = null;
  state.netDetailError = "";
  state.netPool = null;
  state.treeRev = 0;
  state.seenRev = -1;
}

function fail(e: unknown): string {
  const text = message(e);
  if (text === "attachment changed") return text;
  state.error = text;
  if (/cannot reach/.test(text)) state.status = "lost";
  return text;
}

function streams(): PushStream[] {
  const want: PushStream[] = ["status"];
  if (state.consoleOpen) want.push("logs");
  if (state.tab === "elements") want.push("tree");
  if (state.tab === "network") want.push("net");
  if (state.tab === "performance" && !state.captureActive) want.push("perf");
  return want;
}

function openPush(): void {
  if (!state.target) return;
  channel = new Push(state.target, state.logCursor, streams(), {
    onOpen: () => {
      state.pushLive = true;
      state.pushNote = "live";
    },
    onMessage: onPush,
    onClose: (why) => {
      state.pushLive = false;
      state.pushNote = why;
      channel = null;
    },
  });
}

function closePush(): void {
  channel?.close();
  channel = null;
  state.pushLive = false;
  state.pushNote = "";
}

export function resubscribe(): void {
  channel?.subscribe(streams());
}

function onPush(msg: PushMessage): void {
  switch (msg.t) {
    case "hello":
      state.pushNote = `live · proto ${msg.proto}`;
      state.treeRev = msg.treeRev;
      break;
    case "logs":
      for (const entry of msg.entries || []) pushLine(entry);
      state.logCursor = msg.next;
      if (msg.dropped) state.dropped += msg.dropped;
      break;
    case "tree":
      state.treeRev = msg.rev;
      // The tree is REFETCHED only when somebody is looking at it.
      if (state.tab === "elements") scheduleTreeRefresh();
      break;
    case "status":
      state.logStatus = msg.text;
      break;
    case "net":
      for (const entry of msg.entries || []) pushNet(entry);
      if (msg.pool) state.netPool = msg.pool;
      break;
    case "perf":
      applyPerf(msg);
      break;
  }
}

export function pollFallback(): void {
  if (state.status !== "attached" || state.pushLive) return;
  void pollLogs();
  if (state.tab === "elements") void refreshTree();
  if (state.tab === "performance" && !state.captureActive) void refreshPerf();
  if (state.tab === "network") void refreshNet();
}

// windows

export async function refreshWindows(): Promise<void> {
  if (!state.target) return;
  try {
    const data = await send<WindowsReply>(state.target, "windows");
    state.windows = data.windows || [];
    state.inspectors = data.inspectors || [];
    if (state.window !== null && !state.windows.some((w) => w.id === state.window)) {
      state.window = null;
      note({ kind: "dev", text: "the window this inspector was opened for closed — back to the main window" });
    }
  } catch (e) {
    fail(e);
  }
}

export async function selectWindow(id: WindowId | null): Promise<void> {
  if (state.window !== id) {
    inspectionEpoch++;
    state.session++;
    cancelTreeRefresh();
  }
  state.window = id;
  state.selected = null;
  state.styles = null;
  state.rules = null;
  state.expanded = {};
  await refreshTree();
}

export function currentWindow(): WindowInfo | null {
  if (state.window === null) return state.windows.find((w) => w.kind === "main") || null;
  return state.windows.find((w) => w.id === state.window) || null;
}

let treeBusy = false;
let inspectionEpoch = 0;
let treeTimer: ReturnType<typeof setTimeout> | null = null;
let lastTreeRefresh = -Infinity;
const TREE_REFRESH_MS = 200;

function cancelTreeRefresh(): void {
  if (treeTimer !== null) clearTimeout(treeTimer);
  treeTimer = null;
}

function scheduleTreeRefresh(): void {
  if (treeBusy || treeTimer !== null || !state.target || state.tab !== "elements") return;
  treeTimer = setTimeout(
    () => {
      treeTimer = null;
      if (state.target && state.tab === "elements") void refreshTree();
    },
    Math.max(0, lastTreeRefresh + TREE_REFRESH_MS - Date.now()),
  );
}

export async function refreshTree(): Promise<void> {
  if (!state.target || treeBusy) return;
  cancelTreeRefresh();
  treeBusy = true;
  lastTreeRefresh = Date.now();
  const epoch = inspectionEpoch;
  const revision = state.treeRev;
  const target = state.target;
  const window = state.window;
  try {
    const tree = await cmd<TreeNode>("tree", { depth: 14 });
    if (epoch !== inspectionEpoch || target !== state.target || window !== state.window) return;
    state.tree = markRaw(tree);
    state.treeError = "";
    state.seenRev = revision;
    if (!Object.keys(state.expanded).length) expandInitial(tree, 0);
    reindex();
  } catch (e) {
    if (epoch === inspectionEpoch && target === state.target) state.treeError = fail(e);
  } finally {
    treeBusy = false;
    if (
      state.status === "attached" &&
      (epoch !== inspectionEpoch || target !== state.target || window !== state.window || state.treeRev !== revision)
    )
      scheduleTreeRefresh();
  }
}

function expandInitial(node: TreeNode | null, depth: number): void {
  if (!node || depth > 2) return;
  state.expanded[node.id] = true;
  for (const child of node.children || []) expandInitial(child, depth + 1);
}

export function reindex(): void {
  const byId: Record<NodeId, TreeNode> = {};
  const index = (node: TreeNode | null): void => {
    if (!node || node.id === undefined) return;
    byId[node.id] = node;
    for (const child of node.children || []) index(child);
  };
  index(state.tree);

  const query = state.elementSearch.trim().toLowerCase();
  const matches: NodeId[] = [];
  const rows: Row[] = [];
  const walk = (node: TreeNode | null, depth: number): void => {
    if (!node || node.id === undefined) return;
    // Fragment anchors (hidden unless the panel's toggle is on) and whitespace-only text nodes get
    // no row — see `rowVisible`.
    if (!rowVisible(node, state.showAnchors)) return;
    const children = node.children || [];
    const hit = query ? rowText(node).includes(query) : false;
    if (hit) matches.push(node.id);
    rows.push({
      id: node.id,
      depth,
      node,
      children: children.length,
      truncated: node.truncated || 0,
      open: !!state.expanded[node.id],
      hit,
    });
    if (state.expanded[node.id]) for (const child of children) walk(child, depth + 1);
  };
  walk(state.tree, 0);

  state.rows = markRaw(rows);
  state.byId = markRaw(byId);
  state.matches = matches;
  if (state.matchIndex >= matches.length) state.matchIndex = 0;
}

function rowText(node: TreeNode): string {
  const classes = (node.classes || []).join(" ");
  const id = node.props?.id || "";
  return `${node.tag || ""} ${classes} ${id} ${node.text || ""}`.toLowerCase();
}

export function toggle(id: NodeId): void {
  state.expanded[id] = !state.expanded[id];
  reindex();
}

export function toggleAnchors(): void {
  state.showAnchors = !state.showAnchors;
  reindex();
}

export function setExpanded(id: NodeId, open: boolean): void {
  state.expanded[id] = open;
  reindex();
}

export function revealPath(path: NodeId[] | undefined): void {
  if (!Array.isArray(path)) return;
  for (const id of path.slice(0, -1)) state.expanded[id] = true;
  reindex();
}

export function ancestors(id: NodeId | null): TreeNode[] {
  if (id === null || !state.tree) return [];
  const path: TreeNode[] = [];
  const walk = (node: TreeNode, trail: TreeNode[]): boolean => {
    const here = [...trail, node];
    if (node.id === id) {
      path.push(...here);
      return true;
    }
    for (const child of node.children || []) if (walk(child, here)) return true;
    return false;
  };
  walk(state.tree, []);
  return path;
}

export async function select(id: NodeId | null, options: { highlight?: boolean } = {}): Promise<void> {
  state.selected = id;
  state.styles = null;
  state.rules = null;
  state.stylesError = "";
  if (!state.target || id === null || id === undefined) return;
  // A node the tree has never seen — picked out of the target before the next refresh, or selected
  // across a rebuild.
  if (!state.byId[id]) await refreshTree();
  if (options.highlight !== false) highlight(id, SELECT_TTL_MS);
  await loadStyles(id);
}

async function loadStyles(id: NodeId): Promise<void> {
  try {
    state.rules = await cmd<RulesReply>("rules", { node: id });
  } catch {
    state.rules = null;
  }
  try {
    state.styles = await cmd<StylesReply>("styles", { node: id });
  } catch (e) {
    state.stylesError = message(e);
  }
}

export async function refreshStyles(): Promise<void> {
  if (state.selected !== null) await loadStyles(state.selected);
}

export function searchElements(query: string): void {
  state.elementSearch = query;
  state.matchIndex = 0;
  reindex();
}

/**
 * The search runs over the FLATTENED rows, which only contain what is expanded — so a match inside
 * a collapsed subtree would be invisible.
 */
export function stepMatch(delta: number): void {
  const query = state.elementSearch.trim().toLowerCase();
  if (!query || !state.tree) return;
  const all: NodeId[] = [];
  const walk = (node: TreeNode, trail: NodeId[]): void => {
    if (rowText(node).includes(query)) {
      all.push(node.id);
      for (const ancestor of trail) state.expanded[ancestor] = true;
    }
    for (const child of node.children || []) walk(child, [...trail, node.id]);
  };
  walk(state.tree, []);
  reindex();
  if (!all.length) return;
  state.matchIndex = (state.matchIndex + delta + all.length) % all.length;
  const id = all[state.matchIndex]!;
  void select(id);
}

export function highlight(id: NodeId, ttlMs: number): Promise<void> {
  if (!state.target) return Promise.resolve();
  return cmd<unknown>("highlight", { node: id, ttlMs }).then(
    () => {},
    () => {},
  );
}

export function clearHighlight(): Promise<void> {
  if (!state.target) return Promise.resolve();
  return cmd<unknown>("highlight", { clear: true }).then(
    () => {},
    () => {},
  );
}

let pickBusy = false;
let pickMode = 0;
export async function cancelPick(): Promise<void> {
  const target = state.target;
  pickMode++;
  state.picking = false;
  state.pickInfo = null;
  if (target) await send(target, "pick", { op: "stop" }).catch(() => {});
}

export async function togglePick(): Promise<void> {
  if (state.picking) return cancelPick();
  if (!state.target || state.status !== "attached") return;
  const target = state.target,
    epoch = inspectionEpoch,
    mode = ++pickMode;
  state.picking = true;
  state.pickInfo = null;
  try {
    await send(target, "pick", { op: "start" });
    if (epoch !== inspectionEpoch || mode !== pickMode) return;
    await pickStep();
  } catch (error) {
    if (epoch === inspectionEpoch && mode === pickMode) {
      state.picking = false;
      fail(error);
    }
  }
}

export async function pickStep(): Promise<void> {
  if (!state.picking || !state.target || pickBusy) return;
  pickBusy = true;
  const target = state.target,
    epoch = inspectionEpoch,
    mode = pickMode;
  try {
    const info = await send<PickReply & { active: boolean; selected: boolean; cancelled: boolean }>(
      target,
      "pick",
      { op: "status" },
      3000,
      { priority: "background", isCurrent: () => epoch === inspectionEpoch && mode === pickMode },
    );
    if (epoch !== inspectionEpoch || mode !== pickMode) return;
    state.pickInfo = markRaw(info);
    if (info.active) return;
    state.picking = false;
    if (!info.selected || !info.node) return;
    if (info.window !== undefined) {
      const pickedWindow = state.windows.find((w) => w.id === info.window);
      const next = pickedWindow?.kind === "main" ? null : info.window;
      if (state.window !== next) {
        state.window = next;
        state.session++;
      }
    }
    state.tab = "elements";
    resubscribe();
    await refreshTree();
    if (epoch !== inspectionEpoch || mode !== pickMode) return;
    revealPath(info.path);
    await select(info.node.id);
    note({ kind: "dev", text: `picked ${nodeLabel(info.node)} (node ${info.node.id})` });
  } catch (error) {
    if (epoch === inspectionEpoch && mode === pickMode) {
      state.picking = false;
      fail(error);
    }
  } finally {
    pickBusy = false;
  }
}

let logsBusy = false;

function note(entry: { kind: string; text: string; seq?: number; at?: number }): void {
  pushLine({ seq: -1, at: Date.now(), ...entry });
}

let lineUid = 0;

function pushLine(entry: ConsoleLine): void {
  entry.uid = ++lineUid;
  state.logs.push(entry);
  if (state.logs.length > LOG_LIMIT) state.logs.splice(0, state.logs.length - LOG_LIMIT);
}

export async function pollLogs(): Promise<void> {
  if (!state.target || logsBusy || state.status !== "attached") return;
  logsBusy = true;
  try {
    const data = await cmd<LogsReply>("logs", { since: state.logCursor });
    state.logCursor = data.next;
    state.logStatus = data.status || "";
    if (data.dropped) state.dropped += data.dropped;
    for (const entry of data.entries || []) pushLine(entry);
  } catch (e) {
    fail(e);
  } finally {
    logsBusy = false;
  }
}

export function visibleLogs(): ConsoleLine[] {
  const needle = state.logFilter.trim().toLowerCase();
  return state.logs.filter((line) => {
    // Lines the inspector wrote about itself are never filtered out by level: "attached to …" and
    // "picked div.card" are the transcript of what you did.
    const own = line.seq < 0 && (line.kind === "input" || line.kind === "result" || line.kind === "dev");
    if (!own && !state.levels[levelOf(line.kind)]) return false;
    if (needle && !line.text.toLowerCase().includes(needle)) return false;
    return true;
  });
}

export async function runEval(source: string): Promise<void> {
  if (!state.target || !source.trim()) return;
  note({ kind: "input", text: source });
  if (state.evalHistory[state.evalHistory.length - 1] !== source) state.evalHistory.push(source);
  state.evalIndex = state.evalHistory.length;
  try {
    const data = await cmd<EvalReply>("eval", { js: source });
    if (data.exception) {
      note({ kind: "error", text: `${data.exception}${data.stack ? `\n${data.stack}` : ""}` });
    } else {
      note({ kind: "result", text: `${format(data.value)}  ·  ${data.type}` });
    }
  } catch (e) {
    note({ kind: "error", text: fail(e) });
  }
}

function format(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export function clearConsole(): void {
  state.logs = [];
  state.dropped = 0;
}

/**
 * One `eval` per prefix change would be one target frame per keystroke, so the name list is
 * fetched ONCE per attach and filtered locally.
 */
let globalNames: string[] = [];

export async function loadCompletions(): Promise<void> {
  if (!state.target || globalNames.length) return;
  try {
    const data = await cmd<EvalReply>("eval", {
      js: "Object.getOwnPropertyNames(globalThis).filter(function(n){return /^[A-Za-z_$][\\w$]*$/.test(n)}).sort()",
    });
    if (Array.isArray(data.value)) globalNames = data.value as string[];
  } catch {
    globalNames = [];
  }
}

export function complete(prefix: string): string[] {
  const word = /[\w$.]*$/.exec(prefix)?.[0] || "";
  if (!word || word.includes(".")) return [];
  const lower = word.toLowerCase();
  return globalNames.filter((n) => n.toLowerCase().startsWith(lower)).slice(0, 8);
}

function applyPerf(data: PerfReply): void {
  state.perf = data;
  const samples = data.samples || [];
  // The reply's `samples` are the target's ring (which overlaps between pushes), so appending all
  // of them would draw the same milliseconds several times.
  if (samples.length) state.perfHistory.push(samples[samples.length - 1]!);
  if (state.perfHistory.length > PERF_HISTORY) {
    state.perfHistory.splice(0, state.perfHistory.length - PERF_HISTORY);
  }
  // A cold chart (the panel was just opened) is seeded from the ring so it has a shape immediately
  // instead of drawing itself in over a minute.
  if (state.perfHistory.length < 8 && samples.length > 8) {
    state.perfHistory = samples.slice(-PERF_HISTORY);
  }
}

export async function refreshPerf(): Promise<void> {
  if (!state.target) return;
  try {
    applyPerf(await cmd<PerfReply>("perf"));
  } catch (e) {
    fail(e);
  }
}

let memInFlight = false;

function applyMem(reply: MemReply | MemUnsupported): string | null {
  const unsupported = memUnsupported(reply);
  if (unsupported !== null || !("breakdown" in reply)) {
    state.mem = null;
    state.memError = "";
    state.memUnsupported = unsupported ?? "";
    return unsupported;
  }
  state.mem = reply.breakdown;
  state.memCollectMs = reply.collectMs;
  state.memError = "";
  state.memUnsupported = "";
  state.memHistory.push(sampleOf(reply.breakdown, Date.now()));
  if (state.memHistory.length > MEM_HISTORY) state.memHistory.splice(0, state.memHistory.length - MEM_HISTORY);
  return null;
}

export async function refreshMem(): Promise<void> {
  if (!state.target || memInFlight || state.memGcBusy || state.captureActive) return;
  const epoch = inspectionEpoch;
  memInFlight = true;
  try {
    const reply = await cmd<MemReply | MemUnsupported>("mem");
    if (epoch === inspectionEpoch) applyMem(reply);
  } catch (e) {
    if (epoch === inspectionEpoch) state.memError = message(e);
  } finally {
    memInFlight = false;
  }
}

export async function measureResident(): Promise<void> {
  if (!state.target || state.memResidentBusy) return;
  state.memResidentBusy = true;
  try {
    const reply = await cmd<MemReply | MemUnsupported>("mem", { regions: true }, 60_000);
    const unsupported = applyMem(reply);
    if (unsupported !== null || !("breakdown" in reply)) flash(`RAM breakdown unavailable: ${unsupported}`, true);
    else state.memResident = { at: Date.now(), breakdown: reply.breakdown };
  } catch (e) {
    flash(`RAM breakdown failed: ${message(e)}`, true);
  } finally {
    state.memResidentBusy = false;
  }
}

export async function forceGc(): Promise<void> {
  if (!state.target || state.memGcBusy) return;
  state.memGcBusy = true;
  try {
    const reply = await cmd<MemReply | MemUnsupported>("mem", { gc: true }, 60_000);
    // A host without a port did not collect either: it never reads `gc`.
    const unsupported = applyMem(reply);
    if (unsupported !== null || !("breakdown" in reply)) flash(`Force GC unavailable: ${unsupported}`, true);
    else if (reply.gc) flash(gcSummary(reply.gc));
  } catch (e) {
    flash(`Force GC failed: ${message(e)}`, true);
  } finally {
    state.memGcBusy = false;
  }
}

function pushNet(entry: NetEntry): void {
  state.net.push(entry);
  state.netCursor = Math.max(state.netCursor, entry.seq + 1);
  if (state.net.length > NET_LIMIT) state.net.splice(0, state.net.length - NET_LIMIT);
}

export async function refreshNet(): Promise<void> {
  if (!state.target) return;
  try {
    const data = await cmd<NetReply>("net", { since: state.netCursor });
    for (const entry of data.entries || []) pushNet(entry);
    state.netCursor = data.next;
    if (data.pool) state.netPool = data.pool;
  } catch (e) {
    fail(e);
  }
}

export function clearNet(): void {
  state.net = [];
  closeNetDetail();
}

/** On demand, one command per click: a body never rides the push feed. */
export function selectNet(seq: number | null): Promise<void> {
  ui.copyTarget = "network";
  if (seq === state.netSelected && state.netDetail) return Promise.resolve();
  const entry = seq === null ? undefined : state.net.find((e) => e.seq === seq);
  if (entry?.kind === "reload") return Promise.resolve();
  state.netSelected = seq;
  state.netDetail = null;
  state.netDetailError = "";
  if (seq === null) return Promise.resolve();
  if (entry?.kind === "ws") state.netTab = "socket";
  else if (state.netTab === "socket") state.netTab = "headers";
  return loadNetDetail(seq);
}

export function closeNetDetail(): void {
  state.netSelected = null;
  state.netDetail = null;
  state.netDetailError = "";
}

/**
 * The response body is read off a CLONE after the row was recorded (`net.rs` TEE_JS), so a click
 * on a row that has only just appeared can land before the body does.
 */
export async function loadNetDetail(seq: number, attempt = 0): Promise<void> {
  if (!state.target) return;
  state.netDetailBusy = true;
  try {
    const detail = await detailFor(seq);
    if (state.netSelected !== seq) return;
    state.netDetail = detail;
    state.netDetailError = "";
    const res = detail.response;
    const waiting =
      (detail.entry.kind !== "ws" && res !== undefined && res.body === null && !detail.error) ||
      !!detail.cookies?.pending;
    if (waiting && attempt < 4) {
      setTimeout(
        () => {
          if (state.netSelected === seq) void loadNetDetail(seq, attempt + 1);
        },
        250 * (attempt + 1),
      );
    }
  } catch (e) {
    if (state.netSelected !== seq) return;
    const text = message(e);
    state.netDetailError = /unknown command/.test(text)
      ? "this target predates net_detail: it records the rows, not what is behind them"
      : text;
    if (/cannot reach/.test(text)) fail(e);
  } finally {
    state.netDetailBusy = false;
  }
}

export function detailFor(seq: number): Promise<NetDetail> {
  if (state.netDetail && state.netDetail.entry.seq === seq) return Promise.resolve(state.netDetail);
  return cmd<NetDetail>("net_detail", { seq });
}

// They also reach what a drag cannot: a whole message behind its ellipsis, a body past what the
// virtual list has on screen.

let flashTimer: ReturnType<typeof setTimeout> | null = null;

export function flash(text: string, bad = false): void {
  ui.flash = text;
  ui.flashBad = bad;
  if (flashTimer) clearTimeout(flashTimer);
  flashTimer = setTimeout(() => {
    ui.flash = "";
    flashTimer = null;
  }, 4000);
}

/**
 * Puts `text` on the system clipboard through the app's own `navigator.clipboard` — redjs's async
 * Clipboard API over the host clipboard, the same one a native `<input>` uses for Ctrl+C.
 */
export async function copyText(text: string, what: string): Promise<boolean> {
  const clipboard = globalThis.navigator?.clipboard;
  if (!clipboard || typeof clipboard.writeText !== "function") {
    flash("no clipboard in this host — vn-app was built without its `clipboard` feature", true);
    return false;
  }
  try {
    await clipboard.writeText(text);
    flash(`copied ${what}`);
    return true;
  } catch (e) {
    flash(`copy failed: ${message(e)}`, true);
    return false;
  }
}

export function openMenu(x: number, y: number, items: MenuItem[]): void {
  ui.menu = { x, y, items };
}

export function closeMenu(): void {
  ui.menu = null;
}

export function visibleNet(): NetEntry[] {
  const needle = state.netFilter.trim().toLowerCase();
  if (!needle) return state.net;
  return state.net.filter((e) => e.kind === "reload" || e.url.toLowerCase().includes(needle));
}

export async function refreshStorage(): Promise<void> {
  if (!state.target) return;
  state.storageError = "";
  try {
    const data = await cmd<StorageReply>("storage", { area: state.storageArea, op: "list" });
    state.storage = data.entries || [];
  } catch (e) {
    state.storage = [];
    state.storageError = message(e);
  }
}

export async function setStorage(key: string, value: string): Promise<void> {
  if (!state.target || !key) return;
  try {
    await cmd<StorageReply>("storage", { area: state.storageArea, op: "set", key, value });
    await refreshStorage();
  } catch (e) {
    state.storageError = message(e);
  }
}

export async function removeStorage(key: string): Promise<void> {
  if (!state.target) return;
  try {
    await cmd<StorageReply>("storage", { area: state.storageArea, op: "remove", key });
    await refreshStorage();
  } catch (e) {
    state.storageError = message(e);
  }
}

export async function clearStorage(): Promise<void> {
  if (!state.target) return;
  try {
    await cmd<StorageReply>("storage", { area: state.storageArea, op: "clear" });
    await refreshStorage();
  } catch (e) {
    state.storageError = message(e);
  }
}

/**
 * A live preview would mean a full window readback plus a PNG encode per frame in the target — the
 * most expensive command on the wire by two orders of magnitude.
 */
export async function capture(): Promise<void> {
  if (!state.target || state.shotBusy) return;
  state.shotBusy = true;
  state.shotError = "";
  try {
    const shot = await cmd<ScreenshotReply>("screenshot", {}, 20000);
    state.shot = {
      ...shot,
      at: Date.now(),
      url: `file:///${String(shot.path).replace(/\\/g, "/")}`,
    };
  } catch (e) {
    state.shotError = fail(e);
  } finally {
    state.shotBusy = false;
  }
}

export function setTab(tab: Tab): void {
  state.tab = tab;
  resubscribe();
  if (tab === "storage") void refreshStorage();
  if (tab === "performance") {
    void refreshPerf();
    void refreshMem();
  }
  if (tab === "network") void refreshNet();
  if (tab === "targets") void refreshWindows();
  if (tab === "elements" && state.treeRev !== state.seenRev) void refreshTree();
}

export function toggleConsole(): void {
  state.consoleOpen = !state.consoleOpen;
  resubscribe();
}

export function panelRequest(): <T>(payload: Record<string, unknown>) => Promise<T> {
  const target = state.target,
    window = state.window,
    session = state.session;
  return async <T>(payload: Record<string, unknown>): Promise<T> => {
    if (!target) throw new Error("not attached to a target");
    const { cmd: name, ...args } = payload;
    if (typeof name !== "string") throw new Error("missing inspector command");
    const current = () => state.session === session && state.target?.port === target.port;
    if (!current()) throw new Error("attachment changed");
    const leased =
      (name === "trace" && args.op === "recording-start") || (name === "allocations" && args.op === "start");
    const scoped = { ...(window === null ? {} : { window }), ...args, ...(leased ? { leaseMs: 15_000 } : {}) };
    const reply = await send<T>(target, name, scoped, undefined, { isCurrent: current });
    if (!current()) throw new Error("attachment changed");
    return markRaw(reply as object) as T;
  };
}
export function setCaptureActive(active: boolean): void {
  state.captureActive = active;
  resubscribe();
}

export function captureHeartbeat(): void {
  if (!state.target || !state.captureActive) return;
  void cmd("capture-heartbeat").catch(() => {});
}
