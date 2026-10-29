// The shapes the devtools wire answers with — docs/DEVTOOLS.md.
//
// Written from `crates/vn-app/src/devtools/mod.rs`'s own `Json::obj(...)`
// builders, one interface per `cmd_*`, and OPTIONAL exactly where the Rust
// pushes an entry conditionally. That distinction is the whole value of this
// file: `classes`, `props`, `text`, `truncated`, `media`, `pseudo` and
// `diagnostics` are all absent rather than empty when there is nothing to say,
// so a reader that does not guard is reading `undefined` — which in JavaScript
// looked exactly like reading an empty one.
//
// Nothing here is shared with `packages/vn-test`: an app bundle must not import
// the test harness. Keep the two in step; `docs/DEVTOOLS.md` is the contract
// both are written from.

/** A node id in the TARGET's retained tree. Ids are the target's, not ours. */
export type NodeId = number;

/** `(x, y, w, h)` in the target's CSS pixels; `null` when it was not painted. */
export type Bounds = { x: number; y: number; w: number; h: number } | null;

/** A gpui window id in the target process. `windows` lists them. */
export type WindowId = number;

// ---------------------------------------------------------------------------
// discovery — `__vn_devtools_targets()`
// ---------------------------------------------------------------------------

/** One attachable instance: a `<slot>/devtools.json` whose port answered. */
export interface Target {
  /** The slot directory's name — what the picker shows. */
  slot: string;
  dir: string;
  port: number;
  pid: number;
  /** The handshake token every command must carry. */
  token: string;
  /** Did the port accept a connection just now? A stale handshake reads false. */
  alive: boolean;
  /** THIS process. Attachable, and a legitimate thing to ask for by hand. */
  self: boolean;
}

export interface TargetList {
  /** The slot `--target` named (`VN_DEVTOOLS_TARGET`), or `null`. */
  preferred: string | null;
  self: { pid: number; dir: string } | null;
  targets: Target[];
  /** Set by `wire.ts` when the host fn is missing or its answer did not parse. */
  error?: string;
}

/**
 * `globalThis.__VN_DEVTOOLS_ARGS__` — published by the host when this process
 * was started with `--devtools` (`crates/vn-app/src/inspector.rs`).
 *
 * It is how an inspector opened by **F12** knows which WINDOW of the target it
 * was opened for, which is a fact no amount of scanning can recover: one
 * process can paint several windows off one tree (docs/MULTIWINDOW.md), and the
 * key that was pressed happened in exactly one of them.
 */
export interface DevtoolsArgs {
  target: string | null;
  window: WindowId | null;
  parent: number | null;
  embedded: boolean;
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

export interface PingReply {
  pid: number;
  [key: string]: unknown;
}

/** One node of the `tree` reply. `children` and `truncated` are exclusive. */
export interface TreeNode {
  id: NodeId;
  /** `"element"` | `"text"` | `"anchor"` — `vn_tree::devtools::node_info`. */
  kind: string;
  tag: string;
  classes?: string[];
  props?: Record<string, string>;
  scopes?: string[];
  listeners?: string[];
  text?: string;
  bounds: Bounds;
  children?: TreeNode[];
  /** How many children were NOT sent, because `depth` ran out. */
  truncated?: number;
}

export interface StylesReply {
  id: NodeId;
  hovered: boolean;
  /** Bucket name (`base`, `live`, `hover`, …) -> the pretty `Debug` dump, one
   *  array entry per SOURCE LINE. The engine's vocabulary, not CSS's. */
  computed: Record<string, string[]>;
}

export interface Declaration {
  property: string;
  value: string;
  /** Absent rather than `false` when the declaration is not `!important`. */
  important?: boolean;
}

export interface Rule {
  selector: string;
  specificity: number;
  /** Source order. With `specificity` it is the cascade's whole sort key. */
  order: number;
  /** The state bucket this rule folds into: `base`, `hover`, `active`, … */
  state: string;
  declarations: Declaration[];
  /** The `@media` prelude the rule came from, when it came from one. */
  media?: string;
  /** `.group:hover .this` — the state is an ANCESTOR's, not this node's. */
  ancestorState?: boolean;
  pseudo?: string;
  /** vn-css's advisories for THIS rule's declarations — the field a browser
   *  has no equivalent for. */
  diagnostics?: string[];
}

export interface RulesReply {
  id: NodeId;
  count: number;
  rules: Rule[];
  /** Present when the sheet was compiled with rule-text capture off: the
   *  selectors and cascade order are still exact, the declarations are not. */
  note?: string;
}

export interface PickReply {
  x: number;
  y: number;
  /** gpui's own `Window::is_window_hovered`. A synthesized pointer does not
   *  set it — only a real cursor — which is what makes the latch reliable. */
  insideWindow: boolean;
  /** Which window this answer is about. v2: `pick` with no `window` argument
   *  asks every window and reports the one the pointer is actually in. */
  window: WindowId;
  viewport: { w: number; h: number };
  /** Root-first ancestor chain of the node under the pointer. */
  path: NodeId[];
  node: TreeNode | null;
}

/** One `%c` run: the CSS the app passed, and where in `text` it starts. */
export interface StyleRun {
  at: number;
  css: string;
}

/** One line of the target's log ring. `kind` is `log`/`warn`/`css`/… */
export interface LogEntry {
  seq: number;
  /** Milliseconds since the target's log ring was enabled. */
  at: number;
  kind: string;
  text: string;
  /** `console.log("%cA%cB", cssA, cssB)`. `null` for every other line —
   *  which is nearly all of them. */
  styles?: StyleRun[] | null;
}

export interface LogsReply {
  entries: LogEntry[];
  /** The cursor to pass as `since` next time. */
  next: number;
  status: string | null;
  /** How many entries the ring dropped before we asked. Absent when none. */
  dropped?: number;
}

export interface EvalReply {
  value?: unknown;
  type?: string;
  /** A throw inside the TARGET is a successful round trip, reported here. */
  exception?: string;
  stack?: string;
}

export interface ScreenshotReply {
  path: string;
  width: number;
  height: number;
  hwnd: string;
  backend: string;
  clientX?: number;
  clientY?: number;
  scale?: number;
  /** Present ONLY when the readback found transparent pixels — a mica/acrylic
   *  backdrop, which DWM composites under the app where the capture cannot see
   *  it. Absent, not `false`, when the capture is opaque. */
  hasAlpha?: boolean;
  note?: string;
}

// ---------------------------------------------------------------------------
// v2 commands
// ---------------------------------------------------------------------------

/** One OS window of the target process. */
export interface WindowInfo {
  id: WindowId;
  /** `"main"` or `"popout"`. */
  kind: string;
  title: string;
  /** The node this window paints as its root; `0` = the whole document. */
  root: NodeId;
  viewport: { w: number; h: number };
  scale: number;
  focused: boolean;
  hovered: boolean;
}

export interface WindowsReply {
  windows: WindowInfo[];
  pid: number;
  /** The inspectors the TARGET has open on itself (F12). */
  inspectors: { window: WindowId | null; pid: number }[];
}

export interface PerfReply {
  frames: number;
  uptimeMs: number;
  /** `main` → first frame. `null` before the first frame happened. */
  bootMs: number | null;
  avgMs: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  fps: number;
  /**
   * One frame's budget — the target's display refresh interval. `null` when the
   * platform does not report it (and absent from a target older than this field).
   */
  budgetMs?: number | null;
  /** The tail of the rolling frame-interval ring, oldest first. */
  samples: number[];
  /** Live nodes in the retained tree. */
  nodes: number;
  windows: number;
  /** Is `VN_PERF` on in the target? It owns the per-phase counters. */
  phases: boolean;
}

/** One exchange the network tee recorded (`crates/vn-app/src/devtools/net.rs`). */
export interface NetEntry {
  seq: number;
  /**
   * The exchange's record id. Present when the host kept DETAILS for it
   * (`net_detail`); absent from a host older than m26, or for a request that
   * started while the devtools were off. A socket's rows all share one.
   */
  rid?: number;
  /**
   * `"fetch"`, `"xhr"` or `"ws"`, or `"reload"`: a marker row the host put in
   * where it ran the app again in the same realm (`cause` says why). A marker
   * has no record behind it and an empty `url`. The rows before it are the
   * previous run's.
   */
  kind: string;
  url: string;
  /** A reload marker only: why the host ran the app again, e.g. `"location.reload()"`. */
  cause?: string;
  /** fetch only. */
  method?: string;
  /** Milliseconds since the epoch, as the TARGET measured it. */
  at: number;
  ms?: number;
  status?: number;
  ok?: boolean;
  type?: string;
  error?: string;
  /** ws only: `open-requested` | `open` | `error` | `close`. */
  phase?: string;
  sent?: number;
  received?: number;
  /**
   * The connection, spliced on by the host for a response that came off the
   * network (redjs `fetch::net_info`): `h2` / `http/1.1`, a process-unique
   * connection id (equal ids shared one connection), and whether it had
   * carried an earlier request. Absent for `data:`, a failure, a socket, XHR.
   */
  proto?: string;
  conn?: number;
  reused?: boolean;
}

/** The pooled transport's counters (redjs `http::pool::stats`). */
export interface PoolStats {
  /** Connections opened, by protocol. */
  h1: number;
  h2: number;
  /** Attempts put on a connection — a retried request counts twice. */
  requests: number;
  /** …of which on a connection that had carried an earlier one. */
  reused: number;
  retries: number;
}

export interface NetReply {
  entries: NetEntry[];
  next: number;
  dropped: number;
  /** Absent from a host older than the pooled transport. */
  pool?: PoolStats;
}

/** `[name, value]`, in the order the app (or the server) gave them. */
export type HeaderPair = [string, string];

/**
 * A captured body — `net.rs` `body_json`. `data` is absent when nothing was
 * read (`note` says why) or the bytes were `evicted` to stay in the budget.
 */
export interface NetBody {
  /** `utf8`: `data` is the text. `base64`: `data` is the bytes (binary types). */
  encoding?: "utf8" | "base64";
  data?: string;
  /** Bytes kept — at most `limits.bodyCap`. */
  stored: number;
  /** Fewer bytes are kept than the body had. */
  truncated?: boolean;
  /** The whole body's size when known: bytes, or UTF-16 units when `unit` is `chars`. */
  total?: number;
  /** How much the tee saw before it stopped reading. */
  seen?: number;
  complete?: boolean;
  unit?: string;
  mime?: string;
  /** What the request body WAS: text, urlencoded, formdata, blob, binary, stream, request. */
  kind?: string;
  /** Why nothing (or only part) was read. */
  note?: string;
  evicted?: boolean;
}

export interface FormField {
  name: string;
  value?: string;
  file?: { name: string; size?: number; type: string };
}

/** `net_detail <seq>` — everything behind one row (`net.rs` `Ring::detail`). */
export interface NetDetail {
  /** The row that was asked about, as the feed has it. */
  entry: NetEntry;
  /** `null` when the host has no record for the row — `note` says why. */
  detail?: null;
  note?: string;
  rid?: number;
  /** Every row of the exchange: one for a fetch, one per phase for a socket. */
  seqs?: number[];
  kind?: string;
  url?: string;
  method?: string;
  at?: number;
  request?: {
    /** As the APP passed them. The transport's own (Host, Content-Length, …) are not seen by script. */
    headers?: HeaderPair[];
    credentials?: string;
    mode?: string;
    redirect?: string;
    cache?: string;
    bodyInfo?: { kind: string; fields?: FormField[]; implied?: string };
    body?: NetBody | null;
  };
  response?: {
    status?: number;
    statusText?: string;
    type?: string;
    redirected?: boolean;
    url?: string;
    headers?: HeaderPair[];
    /** XMLHttpRequest only. */
    responseType?: string;
    /** `null` until the tee has read it (it reads a clone, after the row was recorded). */
    body?: NetBody | null;
  };
  /** The cookie side — script never sees either header. */
  cookies?: {
    /** The jar's Cookie header for the URL when the request started. */
    sent: string | null;
    /** Set-Cookie lines RECONSTRUCTED from what the jar changed. */
    set: string[];
    /** The final hop's Set-Cookie lines exactly as received, when the response came off the network. */
    wire?: string[];
    pending?: boolean;
    note?: string;
  } | null;
  /**
   * What the transport reports about the connection (redjs `fetch::net_info`)
   * — or, failing that, the protocol from `nextHopProtocol`.
   */
  transport?: { protocol?: string; reused?: boolean; connectionId?: number; remote?: string | null };
  timing?: { headersMs?: number; bodyMs?: number; failedMs?: number };
  error?: { name?: string; message?: string };
  ws?: {
    protocols?: string[];
    protocol?: string;
    extensions?: string;
    binaryType?: string;
    openMs?: number;
    closeMs?: number;
    errorMs?: number;
    closeCode?: number;
    closeReason?: string;
    wasClean?: boolean;
    error?: boolean;
    sent?: number;
    received?: number;
    sentBytes?: number;
    receivedBytes?: number;
    lastAt?: number;
  };
  metaTruncated?: boolean;
  limits?: { bodyCap: number; budget: number; used: number; evicted: number };
}

/** The tabs of the request details pane. */
export type NetTab = "headers" | "payload" | "response" | "timing" | "socket";

export interface StorageReply {
  area: string;
  length?: number;
  entries?: { key: string; value: string }[];
  key?: string;
  value?: string;
  removed?: boolean;
  cleared?: boolean;
}

// ---------------------------------------------------------------------------
// the push channel — `crates/vn-app/src/devtools/ws.rs`
// ---------------------------------------------------------------------------

export interface PushHello {
  t: "hello";
  proto: number;
  pid: number;
  logCursor: number;
  treeRev: number;
}
export interface PushLogs extends LogsReply {
  t: "logs";
}
export interface PushTree {
  t: "tree";
  rev: number;
}
export interface PushStatus {
  t: "status";
  text: string;
}
export interface PushNet {
  t: "net";
  entries: NetEntry[];
  pool?: PoolStats;
}
export interface PushPerf extends PerfReply {
  t: "perf";
}

export type PushMessage = PushHello | PushLogs | PushTree | PushStatus | PushNet | PushPerf;

/** Which streams a socket wants. The host defaults to all of them. */
export type PushStream = "logs" | "perf" | "tree" | "net" | "status";

// ---------------------------------------------------------------------------
// the store's own shapes
// ---------------------------------------------------------------------------

/** A flattened, on-screen row of the element tree. */
export interface Row {
  id: NodeId;
  depth: number;
  node: TreeNode;
  children: number;
  truncated: number;
  open: boolean;
  /** Set by a search: this row matched the query. */
  hit?: boolean;
}

/** A console line. `seq` is `-1` for lines this inspector wrote itself. */
export interface ConsoleLine {
  seq: number;
  at: number;
  kind: string;
  text: string;
  styles?: StyleRun[] | null;
  /**
   * This inspector's own id for the line, stamped when it is pushed. Selection
   * and expansion key on it: an index moves every time the 1000-line buffer
   * drops its oldest, and `seq` is `-1` for every line the inspector wrote.
   */
  uid?: number;
}

/** One entry of the right-click menu (`components/ContextMenu.vue`). */
export interface MenuItem {
  label: string;
  /** A shortcut or a count, drawn dim on the right. */
  hint?: string;
  disabled?: boolean;
  /** Draw a divider ABOVE this item. */
  divider?: boolean;
  run: () => void;
}

/** A capture, plus when it was taken and where the pane should load it from. */
export interface Shot extends ScreenshotReply {
  at: number;
  url: string;
}

/** "detached" -> "attaching" -> "attached", or "lost" when the wire dies. */
export type Status = "detached" | "attaching" | "attached" | "lost";

/** Which panel is in the main area. */
export type Tab = "elements" | "network" | "performance" | "storage" | "targets";
