// All inspector state, and every command that changes it.
//
// One plain `reactive` module rather than pinia: there is exactly one inspector
// per window, nothing to inject, and every panel is a view over one object.
// (pinia works here — apps/hello proves it — it just buys nothing.)
//
// ## The budget rule, restated for v2
//
// Every COMMAND costs the target a frame, so commands are rare, serialized
// (`wire.ts`) and skipped while a previous one is in flight. Everything that
// used to be a POLL is now a push (`Push`, `crates/vn-app/src/devtools/ws.rs`):
// logs, the tree revision, frame stats and network entries arrive when they
// happen. What is left on a timer is only what genuinely cannot be pushed —
// the picker (it is a question about the pointer, asked while picking) and the
// target scan (a local directory walk that involves no target at all).
//
//   | v1                        | v2                                        |
//   |---------------------------|-------------------------------------------|
//   | logs   700 ms poll        | pushed, same frame                        |
//   | tree  2500 ms poll        | pushed REVISION; refetched only if the    |
//   |                           | Elements panel is open and it moved       |
//   | picker 150 ms poll        | unchanged — it is a pointer question       |
//   | targets 3000 ms           | unchanged — no target is involved          |

import { markRaw, reactive } from "vue";
import { Push, devtoolsArgs, listTargets, message, send } from "./wire";
import { nodeLabel, rowVisible } from "./dump";
import { gcSummary, sampleOf, type MemSample } from "./memfmt";
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

/** How long the target keeps a picker highlight without being told again. */
const PICK_TTL_MS = 1500;
/** …and how long a SELECTION highlight lives; refreshed while it stays selected. */
const SELECT_TTL_MS = 6000;
const LOG_LIMIT = 1000;
const NET_LIMIT = 400;
/** How many frame samples the Performance panel keeps for its chart. */
const PERF_HISTORY = 240;
/** …and how many memory samples (one per `mem` poll, every 2 s: four minutes). */
const MEM_HISTORY = 120;
/** The memory poll's period while the Performance panel is open. */
export const MEM_POLL_MS = 2000;

/** The console's severity buckets, in the order the filter bar shows them. */
export const LEVELS = ["error", "warn", "info", "debug"] as const;
export type Level = (typeof LEVELS)[number];

/**
 * A ring `kind` -> a console level.
 *
 * The target's ring uses a coarse CHANNEL name rather than a severity ladder
 * (`vn_tree::log::Entry`), which is the right shape for the ring and the wrong
 * one for a filter bar — so the mapping is here, in the UI, where it belongs.
 * `css` and `advisory` are `warn` because that is what they are: the engine
 * telling you a declaration did not do what you wrote.
 */
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
  // -- attachment
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

  // -- layout
  tab: Tab;
  consoleOpen: boolean;
  treeWidth: number;
  consoleHeight: number;
  dragging: "tree" | "console" | "net" | null;

  // -- elements
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
  /** Draw Vue's fragment anchors as `<!---->` rows. Off: there is one per
   *  `v-if` and one per `v-for`, and none of them paint anything. */
  showAnchors: boolean;

  // -- styles
  styles: StylesReply | null;
  rules: RulesReply | null;
  stylesTab: string;
  stylesError: string;
  copied: string;

  // -- console
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

  // -- picker
  picking: boolean;
  pickInfo: PickReply | null;

  // -- performance
  perf: PerfReply | null;
  perfHistory: number[];
  /** `mem`'s breakdown, polled every 2 s while the Performance panel is open. */
  mem: MemBreakdown | null;
  memHistory: MemSample[];
  memError: string;
  memCollectMs: number;
  /** A `Force GC` is running (it is a full synchronous collection in the target). */
  memGcBusy: boolean;
  /**
   * The last `mem {regions}` — the RAM (working set) per category, which needs
   * a walk of the target's heap and so is taken on request, not polled.
   */
  memResident: { at: number; breakdown: MemBreakdown } | null;
  memResidentBusy: boolean;

  // -- network
  net: NetEntry[];
  netCursor: number;
  netFilter: string;
  /** The row whose details are open (its `seq`), or `null` — the list alone. */
  netSelected: number | null;
  netDetail: NetDetail | null;
  netDetailError: string;
  netDetailBusy: boolean;
  netTab: NetTab;
  /** The request list's width while the details pane is open. */
  netListWidth: number;
  /** The target's pooled-transport counters, as of the newest entries. */
  netPool: PoolStats | null;

  // -- storage
  storageArea: string;
  storage: { key: string; value: string }[];
  storageError: string;

  // -- preview
  shot: Shot | null;
  shotError: string;
  shotBusy: boolean;
}

export const state = reactive<State>({
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

/** How this inspector was started — F12 names the window it was opened for. */
export const args = devtoolsArgs();

/**
 * Cross-panel UI facts that are not inspection state.
 *
 * `typing` is the one that earns its keep: this app is keyboard-first, and
 * every panel's key handler has to know whether the keystroke belongs to a
 * text field instead. A browser would ask `document.activeElement`; the host
 * does not expose one (the focused NODE is known host-side — `mouse` reports
 * it — but not to the app's own JS), so the inputs say so themselves on
 * `@focus` / `@blur`. Two lines per input, and no guessing.
 */
export const ui = reactive({
  typing: false,
  /** Set by the Elements panel so Ctrl+F can reach it from the shell. */
  openElementSearch: null as null | (() => void),
  /** Is that search box open? Escape closes it before it reaches the drawer. */
  elementSearchOpen: false,
  closeElementSearch: null as null | (() => void),
  /** Set by the Console so Ctrl+F and the prompt focus can reach it. */
  focusPrompt: null as null | (() => void),
  /**
   * Which panel Ctrl+C copies from: the one whose row was clicked LAST. Two
   * panels have a selection at once (the console drawer sits under Network),
   * and "the one you touched" is the only answer a person would expect.
   */
  copyTarget: "" as "" | "console" | "network",
  /** The right-click menu, when one is open (`components/ContextMenu.vue`). */
  menu: null as null | { x: number; y: number; items: MenuItem[] },
  /** A one-line acknowledgement in the status bar: "copied 3 lines". */
  flash: "",
  flashBad: false,
});

// ---------------------------------------------------------------------------
// the wire, with the addressed window folded in
// ---------------------------------------------------------------------------

/**
 * A command against the attached target's CURRENTLY SELECTED window.
 *
 * The `window` argument rides on every command rather than being spelled out at
 * each call site: the host reads it uniformly (docs/DEVTOOLS.md), commands that
 * do not care ignore it, and forgetting it at one call site is exactly how a
 * window switcher ends up half-working.
 */
function cmd<T>(name: string, cmdArgs: Record<string, unknown> = {}, timeoutMs?: number): Promise<T> {
  const scoped = state.window === null ? cmdArgs : { window: state.window, ...cmdArgs };
  return send<T>(state.target, name, scoped, timeoutMs);
}

// ---------------------------------------------------------------------------
// attachment
// ---------------------------------------------------------------------------

export async function refreshTargets(): Promise<TargetList> {
  const found = await listTargets();
  state.targets = found.targets || [];
  state.error = found.error || "";
  return found;
}

/**
 * Picks the target `--target` named, else the first live one that is not this
 * process — the inspector attaching to ITSELF on startup would be a confusing
 * default, even though it is a perfectly legal thing to ask for by hand.
 */
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
  closePush();
  state.target = target;
  state.status = target ? "attaching" : "detached";
  state.error = "";
  resetInspection();
  if (!target) return;
  try {
    const pong = await send<PingReply>(target, "ping");
    state.status = "attached";
    state.logCursor = 0;
    note({ kind: "dev", text: `attached to ${target.slot} · pid ${pong.pid} · port ${target.port}` });
    await refreshWindows();
    // F12 opened this inspector for ONE window of the target. Honour that:
    // everything below is then about the window the key was pressed in.
    if (args.window !== null && state.windows.some((w) => w.id === args.window)) {
      state.window = args.window;
    }
    openPush();
    await refreshTree();
    await pollLogs();
    await refreshNet();
  } catch (e) {
    state.status = "lost";
    state.error = message(e);
  }
}

export function detach(): void {
  if (state.target) clearHighlight();
  closePush();
  state.target = null;
  state.status = "detached";
  resetInspection();
}

function resetInspection(): void {
  inspectionEpoch++;
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

/** Every failure funnels through here, so a dead target is reported once. */
function fail(e: unknown): string {
  const text = message(e);
  state.error = text;
  if (/cannot reach/.test(text)) state.status = "lost";
  return text;
}

// ---------------------------------------------------------------------------
// the push channel
// ---------------------------------------------------------------------------

/** Which streams the UI wants right now — the ones a visible panel reads. */
function streams(): PushStream[] {
  const want: PushStream[] = ["logs", "tree", "status", "net"];
  // Frame stats are four messages a second. Asking for them while nobody is
  // looking at the Performance panel is exactly the kind of idle cost this
  // rewrite exists to remove.
  if (state.tab === "performance") want.push("perf");
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

/** Called when the visible panel changes what it needs. */
export function resubscribe(): void {
  channel?.subscribe(streams());
}

function onPush(msg: PushMessage): void {
  switch (msg.t) {
    case "hello":
      // The cursor the socket will stream from. Anything older than this came
      // out of the one `logs` command the attach path runs.
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
      // The tree is REFETCHED only when somebody is looking at it. A revision
      // number is free; a `tree` command is a frame of the target's budget.
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

/** The polling fallback, for a host whose push channel could not be opened. */
export function pollFallback(): void {
  if (state.status !== "attached" || state.pushLive) return;
  void pollLogs();
  if (state.tab === "elements") void refreshTree();
  if (state.tab === "performance") void refreshPerf();
  if (state.tab === "network") void refreshNet();
}

// ---------------------------------------------------------------------------
// windows
// ---------------------------------------------------------------------------

export async function refreshWindows(): Promise<void> {
  if (!state.target) return;
  try {
    const data = await send<WindowsReply>(state.target, "windows");
    state.windows = data.windows || [];
    state.inspectors = data.inspectors || [];
    if (state.window !== null && !state.windows.some((w) => w.id === state.window)) {
      // The window we were pointed at closed. Fall back to the primary one
      // rather than silently addressing nothing.
      state.window = null;
      note({ kind: "dev", text: "the window this inspector was opened for closed — back to the main window" });
    }
  } catch (e) {
    fail(e);
  }
}

export async function selectWindow(id: WindowId | null): Promise<void> {
  state.window = id;
  state.selected = null;
  state.styles = null;
  state.rules = null;
  state.expanded = {};
  await refreshTree();
}

/** The window commands are addressed to, resolved for display. */
export function currentWindow(): WindowInfo | null {
  if (state.window === null) return state.windows.find((w) => w.kind === "main") || null;
  return state.windows.find((w) => w.id === state.window) || null;
}

// ---------------------------------------------------------------------------
// elements
// ---------------------------------------------------------------------------

let treeBusy = false;
let inspectionEpoch = 0;
let treeTimer: ReturnType<typeof setTimeout> | null = null;
let lastTreeRefresh = -Infinity;
const TREE_REFRESH_MS = 200;

function cancelTreeRefresh(): void {
  if (treeTimer !== null) clearTimeout(treeTimer);
  treeTimer = null;
}

/** Collapse a frame-rate revision stream into at most five snapshots a second. */
function scheduleTreeRefresh(): void {
  if (treeBusy || treeTimer !== null || !state.target || state.tab !== "elements") return;
  treeTimer = setTimeout(() => {
    treeTimer = null;
    if (state.target && state.tab === "elements") void refreshTree();
  }, Math.max(0, lastTreeRefresh + TREE_REFRESH_MS - Date.now()));
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
    // Depth 14 covers every app in this repo whole; anything deeper comes back
    // as a `truncated` count and is fetched on demand when the row is opened.
    const tree = await cmd<TreeNode>("tree", { depth: 14 });
    if (epoch !== inspectionEpoch || target !== state.target || window !== state.window) return;
    // Wire snapshots are replaced as a whole. Proxying every node and row
    // adds thousands of reactive reads to each refresh without observing edits.
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
    ) scheduleTreeRefresh();
  }
}

/** Open the first few levels so an attach shows structure, not one root row. */
function expandInitial(node: TreeNode | null, depth: number): void {
  if (!node || depth > 2) return;
  state.expanded[node.id] = true;
  for (const child of node.children || []) expandInitial(child, depth + 1);
}

/**
 * Re-flattens the tree into the visible row list, and indexes ALL of it.
 *
 * The two are separate walks on purpose: `byId` has to answer for a node whose
 * row is collapsed out of view (the styles panel reads its bounds; the picker
 * selects it before its ancestors are open), while `rows` is only what is on
 * screen.
 */
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
    // Fragment anchors (hidden unless the panel's toggle is on) and
    // whitespace-only text nodes get no row — see `rowVisible`. A hidden node's
    // CHILDREN are hidden with it, which is right: an anchor has none, and a
    // text node cannot have any.
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

/** What Ctrl+F searches: the tag, the classes, the id, and any text. */
function rowText(node: TreeNode): string {
  const classes = (node.classes || []).join(" ");
  const id = node.props?.id || "";
  return `${node.tag || ""} ${classes} ${id} ${node.text || ""}`.toLowerCase();
}

export function toggle(id: NodeId): void {
  state.expanded[id] = !state.expanded[id];
  reindex();
}

/** The Elements panel's `<!---->` toggle: show or hide the fragment anchors. */
export function toggleAnchors(): void {
  state.showAnchors = !state.showAnchors;
  reindex();
}

export function setExpanded(id: NodeId, open: boolean): void {
  state.expanded[id] = open;
  reindex();
}

/** Opens every ancestor of `id` so a picked node is actually on screen. */
export function revealPath(path: NodeId[] | undefined): void {
  if (!Array.isArray(path)) return;
  for (const id of path.slice(0, -1)) state.expanded[id] = true;
  reindex();
}

/** The ancestor chain of the selected node, root first — the breadcrumbs. */
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

export async function select(
  id: NodeId | null,
  options: { highlight?: boolean } = {},
): Promise<void> {
  state.selected = id;
  state.styles = null;
  state.rules = null;
  state.stylesError = "";
  if (!state.target || id === null || id === undefined) return;
  // A node the tree has never seen — picked out of the target before the next
  // refresh, or selected across a rebuild. Fetch the tree so the panels have
  // something to name it with.
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
    // A text node has no computed style — that is an answer, not a failure.
    state.stylesError = message(e);
  }
}

/** Re-reads the selected node's styles — the "did my edit land" button. */
export async function refreshStyles(): Promise<void> {
  if (state.selected !== null) await loadStyles(state.selected);
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

export function searchElements(query: string): void {
  state.elementSearch = query;
  state.matchIndex = 0;
  reindex();
}

/**
 * Step to the next (or previous) match, opening whatever is collapsed over it.
 *
 * The search runs over the FLATTENED rows, which only contain what is expanded
 * — so a match inside a collapsed subtree would be invisible. Rather than
 * search the whole tree and then guess which ancestors to open, this expands
 * everything the query hits on the way: `expandTo` walks the real tree.
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
  const id = all[state.matchIndex];
  void select(id);
}

// ---------------------------------------------------------------------------
// highlight + picker
// ---------------------------------------------------------------------------

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

export function togglePick(): void {
  state.picking = !state.picking;
  state.pickInfo = null;
  if (!state.picking) clearHighlight();
}

/**
 * One picker step: ask the target where its pointer is, outline whatever is
 * under it, and LATCH when the pointer leaves the target's window.
 *
 * The latch is the whole design. Two windows, one mouse: the moment you move
 * back to the inspector to click something, the target's pointer is gone — so
 * "gone" is the gesture. Hover the element you want, come back here, and it is
 * selected. (A browser can intercept the click instead because the picker and
 * the page share a window; we do not.)
 *
 * v2: the poll asks with NO window argument, so the host answers for whichever
 * of the target's windows the pointer is actually in — and the picker follows
 * it there, switching the panel's window if you point at a pop-out.
 */
export async function pickStep(): Promise<void> {
  if (!state.picking || !state.target) return;
  let info: PickReply;
  try {
    info = await send<PickReply>(state.target, "pick", {}, 3000);
  } catch (e) {
    fail(e);
    state.picking = false;
    return;
  }
  if (info.insideWindow && info.node) {
    state.pickInfo = info;
    void send(state.target, "highlight", {
      node: info.node.id,
      window: info.window,
      ttlMs: PICK_TTL_MS,
    }).catch(() => {});
    return;
  }
  // Left the window with something under the pointer — that is the pick.
  if (!info.insideWindow && state.pickInfo && state.pickInfo.node) {
    const picked = state.pickInfo;
    state.picking = false;
    state.pickInfo = null;
    // The pointer may have been in a pop-out. Follow it: a picker that selects
    // a node the panel cannot then show is worse than no picker.
    if (picked.window !== undefined && picked.window !== state.window) {
      const target = state.windows.find((w) => w.id === picked.window);
      if (target) {
        state.window = target.kind === "main" ? null : target.id;
        await refreshTree();
      }
    }
    revealPath(picked.path);
    const node = picked.node!;
    await select(node.id);
    note({ kind: "dev", text: `picked ${nodeLabel(node)} (node ${node.id})` });
  }
}

// ---------------------------------------------------------------------------
// console
// ---------------------------------------------------------------------------

let logsBusy = false;

/** A line this inspector wrote about itself. */
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

/** The console rows that pass the level filter and the text filter. */
export function visibleLogs(): ConsoleLine[] {
  const needle = state.logFilter.trim().toLowerCase();
  return state.logs.filter((line) => {
    // Lines the inspector wrote about itself are never filtered out by level:
    // "attached to …" and "picked div.card" are the transcript of what you did.
    const own = line.seq < 0 && (line.kind === "input" || line.kind === "result" || line.kind === "dev");
    if (!own && !state.levels[levelOf(line.kind)]) return false;
    if (needle && !line.text.toLowerCase().includes(needle)) return false;
    return true;
  });
}

/**
 * Evaluates in the TARGET's JSC, in global scope — the console's whole point.
 *
 * A throw inside the app comes back as `{exception, stack}` on a successful
 * round trip (a devtools console reports a failure, it does not fail), so it is
 * pushed as an `error` line rather than raised here.
 */
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
 * Completions for the prompt: the target's own top-level globals.
 *
 * One `eval` per prefix change would be one target frame per keystroke, so the
 * name list is fetched ONCE per attach and filtered locally. That makes it a
 * snapshot rather than a live view — which is the honest trade for a
 * convenience, and is why a name that appears later still evaluates fine.
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

// ---------------------------------------------------------------------------
// performance
// ---------------------------------------------------------------------------

function applyPerf(data: PerfReply): void {
  state.perf = data;
  const samples = data.samples || [];
  // The chart's own history: the newest sample of each push, appended. The
  // reply's `samples` are the target's ring (which overlaps between pushes), so
  // appending all of them would draw the same milliseconds several times.
  if (samples.length) state.perfHistory.push(samples[samples.length - 1]);
  if (state.perfHistory.length > PERF_HISTORY) {
    state.perfHistory.splice(0, state.perfHistory.length - PERF_HISTORY);
  }
  // A cold chart (the panel was just opened) is seeded from the ring so it has
  // a shape immediately instead of drawing itself in over a minute.
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

// ---------------------------------------------------------------------------
// memory
// ---------------------------------------------------------------------------
//
// `mem` is a COMMAND, not a push: it walks the target's address space and asks
// three allocators, which is a few milliseconds of the target's frame — worth
// it every 2 s while somebody is looking at the panel, not 4×/s for everyone.
// A poll that finds the previous one still in flight is skipped.

let memInFlight = false;

function applyMem(reply: MemReply): void {
  state.mem = reply.breakdown;
  state.memCollectMs = reply.collectMs;
  state.memError = "";
  state.memHistory.push(sampleOf(reply.breakdown, Date.now()));
  if (state.memHistory.length > MEM_HISTORY) state.memHistory.splice(0, state.memHistory.length - MEM_HISTORY);
}

export async function refreshMem(): Promise<void> {
  if (!state.target || memInFlight || state.memGcBusy) return;
  memInFlight = true;
  try {
    applyMem(await cmd<MemReply>("mem"));
  } catch (e) {
    // A host older than `mem` answers "unknown command" — said once, in the
    // panel, rather than as the inspector-wide error banner.
    state.memError = message(e);
  } finally {
    memInFlight = false;
  }
}

/**
 * Where the RAM is: `mem {regions}` walks the target's process heap (locked
 * meanwhile — tens of ms on a big heap) so every working-set page can be put
 * in its category. On request only.
 */
export async function measureResident(): Promise<void> {
  if (!state.target || state.memResidentBusy) return;
  state.memResidentBusy = true;
  try {
    const reply = await cmd<MemReply>("mem", { regions: true }, 60_000);
    applyMem(reply);
    state.memResident = { at: Date.now(), breakdown: reply.breakdown };
  } catch (e) {
    flash(`RAM breakdown failed: ${message(e)}`, true);
  } finally {
    state.memResidentBusy = false;
  }
}

/** Two full synchronous JSC collections in the target, then the breakdown. */
export async function forceGc(): Promise<void> {
  if (!state.target || state.memGcBusy) return;
  state.memGcBusy = true;
  try {
    const reply = await cmd<MemReply>("mem", { gc: true }, 60_000);
    applyMem(reply);
    if (reply.gc) flash(gcSummary(reply.gc));
  } catch (e) {
    flash(`Force GC failed: ${message(e)}`, true);
  } finally {
    state.memGcBusy = false;
  }
}

// ---------------------------------------------------------------------------
// network
// ---------------------------------------------------------------------------

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

/**
 * Opens the details pane on row `seq` and asks the target for what is behind
 * it. On demand, one command per click: a body never rides the push feed.
 */
export function selectNet(seq: number | null): Promise<void> {
  ui.copyTarget = "network";
  if (seq === state.netSelected && state.netDetail) return Promise.resolve();
  const entry = seq === null ? undefined : state.net.find((e) => e.seq === seq);
  // A reload marker is a divider with nothing behind it; the selection stays.
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
 * `net_detail` for `seq`, into the pane if that row is still the one open.
 *
 * The response body is read off a CLONE after the row was recorded
 * (`net.rs` TEE_JS), so a click on a row that has only just appeared can land
 * before the body does. That is retried a few times, a little later each
 * time, rather than shown as "no body".
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
      setTimeout(() => {
        if (state.netSelected === seq) void loadNetDetail(seq, attempt + 1);
      }, 250 * (attempt + 1));
    }
  } catch (e) {
    if (state.netSelected !== seq) return;
    const text = message(e);
    state.netDetailError = /unknown command/.test(text)
      ? "this target predates net_detail (m26): it records the rows, not what is behind them"
      : text;
    if (/cannot reach/.test(text)) fail(e);
  } finally {
    state.netDetailBusy = false;
  }
}

/** The details of `seq`, from the pane when it already has them. */
export function detailFor(seq: number): Promise<NetDetail> {
  if (state.netDetail && state.netDetail.entry.seq === seq) return Promise.resolve(state.netDetail);
  return cmd<NetDetail>("net_detail", { seq });
}

// ---------------------------------------------------------------------------
// the clipboard, the right-click menu, the status-bar acknowledgement
// ---------------------------------------------------------------------------
//
// The panes a developer copies from — console messages, the request details,
// a response body — are drag-selectable (the engine's text selection,
// docs/RUNTIME.md §5.17; the rest of the chrome is `user-select: none`, App.vue).
// The explicit copies stay as the shortcuts they are in a browser: a hover
// icon, the right-click menu, and Ctrl+C on the selected rows when no text is
// selected. They also reach what a drag cannot: a whole message behind its
// ellipsis, a body past what the virtual list has on screen.

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
 * Puts `text` on the system clipboard through the app's own
 * `navigator.clipboard` — redjs's async Clipboard API over the host clipboard,
 * the same one a native `<input>` uses for Ctrl+C. `what` is how the status
 * bar names it ("the URL", "3 lines").
 */
export async function copyText(text: string, what: string): Promise<boolean> {
  // Probed, not assumed: `clipboard` needs vn-app's `clipboard` feature, and a
  // host built without it has no `navigator.clipboard` to call.
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
  // Reload markers stay: they say which run a matching row belongs to.
  return state.net.filter((e) => e.kind === "reload" || e.url.toLowerCase().includes(needle));
}

// ---------------------------------------------------------------------------
// storage
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// preview
// ---------------------------------------------------------------------------

/**
 * A screenshot, ON DEMAND only.
 *
 * A live preview would mean a full window readback plus a PNG encode per frame
 * in the target — the most expensive command on the wire by two orders of
 * magnitude. So this is a button, and the pane says when the image was taken.
 * Each capture lands on a new path (the host rotates them), which is also what
 * keeps gpui's image cache from handing back the previous one.
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

// ---------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------

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
}
