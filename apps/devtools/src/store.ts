// All inspector state, and every command that changes it.
//
// One plain `reactive` module rather than pinia: there is exactly one inspector
// per window, nothing to inject, and the four panes are views over one object.
// (pinia works here — apps/hello proves it — it just buys nothing.)
//
// The rule the polling follows: NEVER let a poll outlive its usefulness.
// Every command costs the target a frame, so the loops are slow (logs ~0.7 s,
// tree ~2.5 s), serialized by `wire.ts`, and each one is skipped while the
// previous one is still in flight.

import { reactive } from "vue";
import { listTargets, message, send } from "./wire";
import { nodeLabel } from "./dump";
import type {
  ConsoleLine,
  EvalReply,
  LogsReply,
  NodeId,
  PickReply,
  PingReply,
  Row,
  RulesReply,
  ScreenshotReply,
  Shot,
  Status,
  StylesReply,
  Target,
  TargetList,
  TreeNode,
} from "./protocol";

/** How long the target keeps a picker highlight without being told again. */
const PICK_TTL_MS = 1500;
/** …and how long a SELECTION highlight lives; refreshed while it stays selected. */
const SELECT_TTL_MS = 6000;
const LOG_LIMIT = 500;

interface State {
  // -- attachment
  targets: Target[];
  target: Target | null;
  status: Status;
  error: string;

  // -- elements
  tree: TreeNode | null;
  rows: Row[];
  byId: Record<NodeId, TreeNode>;
  expanded: Record<NodeId, boolean>;
  selected: NodeId | null;
  treeError: string;
  autoTree: boolean;

  // -- styles
  styles: StylesReply | null;
  rules: RulesReply | null;
  stylesTab: string;
  stylesError: string;

  // -- console
  logs: ConsoleLine[];
  logCursor: number;
  logStatus: string;
  dropped: number;
  logFilter: string;

  // -- picker
  picking: boolean;
  pickInfo: PickReply | null;

  // -- preview
  shot: Shot | null;
  shotError: string;
  shotBusy: boolean;
}

export const state = reactive<State>({
  // -- attachment
  targets: [],
  target: null,
  status: "detached",
  error: "",

  // -- elements
  tree: null,
  rows: [],
  byId: {},
  expanded: {},
  selected: null,
  treeError: "",
  autoTree: true,

  // -- styles
  styles: null,
  rules: null,
  stylesTab: "styles",
  stylesError: "",

  // -- console
  logs: [],
  logCursor: 0,
  logStatus: "",
  dropped: 0,
  logFilter: "",

  // -- picker
  picking: false,
  pickInfo: null,

  // -- preview
  shot: null,
  shotError: "",
  shotBusy: false,
});

// ---------------------------------------------------------------------------
// attachment
// ---------------------------------------------------------------------------

export function refreshTargets(): TargetList {
  const found = listTargets();
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
  if (found && found.preferred) {
    const named = live.find((t) => t.dir === found.preferred || t.slot === found.preferred);
    if (named) return named;
  }
  return live.find((t) => !t.self) || live[0] || null;
}

export async function attach(target: Target | null): Promise<void> {
  state.target = target;
  state.status = target ? "attaching" : "detached";
  state.error = "";
  resetInspection();
  if (!target) return;
  try {
    const pong = await send<PingReply>(target, "ping");
    state.status = "attached";
    state.logCursor = 0;
    push({ kind: "dev", text: `attached to ${target.slot} · pid ${pong.pid} · port ${target.port}` });
    await refreshTree();
    await pollLogs();
  } catch (e) {
    state.status = "lost";
    state.error = message(e);
  }
}

export function detach(): void {
  if (state.target) clearHighlight();
  state.target = null;
  state.status = "detached";
  resetInspection();
}

function resetInspection(): void {
  state.tree = null;
  state.rows = [];
  state.byId = {};
  state.expanded = {};
  state.selected = null;
  state.styles = null;
  state.rules = null;
  state.logs = [];
  state.logCursor = 0;
  state.picking = false;
  state.pickInfo = null;
  state.shot = null;
}

/** Every failure funnels through here, so a dead target is reported once. */
function fail(e: unknown): string {
  const text = message(e);
  state.error = text;
  if (/cannot reach/.test(text)) state.status = "lost";
  return text;
}

// ---------------------------------------------------------------------------
// elements
// ---------------------------------------------------------------------------

let treeBusy = false;

export async function refreshTree(): Promise<void> {
  if (!state.target || treeBusy) return;
  treeBusy = true;
  try {
    // Depth 14 covers every app in this repo whole; anything deeper comes back
    // as a `truncated` count and is fetched on demand when the row is opened.
    const tree = await send<TreeNode>(state.target, "tree", { depth: 14 });
    state.tree = tree;
    state.treeError = "";
    if (!Object.keys(state.expanded).length) expandInitial(tree, 0);
    reindex();
  } catch (e) {
    state.treeError = fail(e);
  } finally {
    treeBusy = false;
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

  const rows: Row[] = [];
  const walk = (node: TreeNode | null, depth: number): void => {
    if (!node || node.id === undefined) return;
    // An empty text node is Vue's fragment anchor, not content: it renders
    // nothing in the app and would be pure noise here (there is one per v-for).
    if (node.kind === "text" && !(node.text || "").trim()) return;
    const children = node.children || [];
    rows.push({
      id: node.id,
      depth,
      node,
      children: children.length,
      truncated: node.truncated || 0,
      open: !!state.expanded[node.id],
    });
    if (state.expanded[node.id]) for (const child of children) walk(child, depth + 1);
  };
  walk(state.tree, 0);

  state.rows = rows;
  state.byId = byId;
}

export function toggle(id: NodeId): void {
  state.expanded[id] = !state.expanded[id];
  reindex();
}

/** Opens every ancestor of `id` so a picked node is actually on screen. */
export function revealPath(path: NodeId[] | undefined): void {
  if (!Array.isArray(path)) return;
  for (const id of path.slice(0, -1)) state.expanded[id] = true;
  reindex();
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
  // auto-refresh, or selected across a rebuild. Fetch the tree so the panels
  // have something to name it with.
  if (!state.byId[id]) await refreshTree();
  if (options.highlight !== false) highlight(id, SELECT_TTL_MS);
  await loadStyles(id);
}

async function loadStyles(id: NodeId): Promise<void> {
  try {
    state.styles = await send<StylesReply>(state.target, "styles", { node: id });
  } catch (e) {
    // A text node has no computed style — that is an answer, not a failure.
    state.stylesError = message(e);
  }
  try {
    state.rules = await send<RulesReply>(state.target, "rules", { node: id });
  } catch {
    state.rules = null;
  }
}

/** Re-reads the selected node's styles — the "did my edit land" button. */
export async function refreshStyles(): Promise<void> {
  if (state.selected !== null) await loadStyles(state.selected);
}

// ---------------------------------------------------------------------------
// highlight + picker
// ---------------------------------------------------------------------------

export function highlight(id: NodeId, ttlMs: number): Promise<void> {
  if (!state.target) return Promise.resolve();
  return send<unknown>(state.target, "highlight", { node: id, ttlMs }).then(
    () => {},
    () => {},
  );
}

export function clearHighlight(): Promise<void> {
  if (!state.target) return Promise.resolve();
  return send<unknown>(state.target, "highlight", { clear: true }).then(
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
    highlight(info.node.id, PICK_TTL_MS);
    return;
  }
  // Left the window with something under the pointer — that is the pick.
  if (!info.insideWindow && state.pickInfo && state.pickInfo.node) {
    const picked = state.pickInfo;
    state.picking = false;
    state.pickInfo = null;
    revealPath(picked.path);
    // Narrowed above, and re-read here because `state.pickInfo` was cleared.
    const node = picked.node!;
    await select(node.id);
    push({ kind: "dev", text: `picked ${nodeLabel(node)} (node ${node.id})` });
  }
}

// ---------------------------------------------------------------------------
// console
// ---------------------------------------------------------------------------

let logsBusy = false;

function push(entry: { kind: string; text: string; seq?: number; at?: number }): void {
  state.logs.push({ seq: -1, at: Date.now(), ...entry });
  if (state.logs.length > LOG_LIMIT) state.logs.splice(0, state.logs.length - LOG_LIMIT);
}

export async function pollLogs(): Promise<void> {
  if (!state.target || logsBusy || state.status !== "attached") return;
  logsBusy = true;
  try {
    const data = await send<LogsReply>(state.target, "logs", { since: state.logCursor });
    state.logCursor = data.next;
    state.logStatus = data.status || "";
    if (data.dropped) state.dropped += data.dropped;
    for (const entry of data.entries || []) push(entry);
  } catch (e) {
    fail(e);
  } finally {
    logsBusy = false;
  }
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
  push({ kind: "input", text: source });
  try {
    const data = await send<EvalReply>(state.target, "eval", { js: source });
    if (data.exception) {
      push({ kind: "error", text: `${data.exception}${data.stack ? `\n${data.stack}` : ""}` });
    } else {
      push({ kind: "result", text: `${format(data.value)}  ·  ${data.type}` });
    }
  } catch (e) {
    push({ kind: "error", text: fail(e) });
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
    const shot = await send<ScreenshotReply>(state.target, "screenshot", {}, 20000);
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
