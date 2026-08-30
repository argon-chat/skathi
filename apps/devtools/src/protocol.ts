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

// ---------------------------------------------------------------------------
// discovery — `__vn_devtools_targets()`
// ---------------------------------------------------------------------------

/** One attachable instance: a `<dist>/devtools.json` whose port answered. */
export interface Target {
  /** The dist slot's directory name — what the picker shows. */
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
  viewport: { w: number; h: number };
  /** Root-first ancestor chain of the node under the pointer. */
  path: NodeId[];
  node: TreeNode | null;
}

/** One line of the target's log ring. `kind` is `log`/`warn`/`css`/… */
export interface LogEntry {
  seq: number;
  /** Milliseconds since the epoch, as the TARGET measured it. */
  at: number;
  kind: string;
  text: string;
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
}

/** A console line. `seq` is `-1` for lines this inspector wrote itself. */
export interface ConsoleLine {
  seq: number;
  at: number;
  kind: string;
  text: string;
}

/** A capture, plus when it was taken and where the pane should load it from. */
export interface Shot extends ScreenshotReply {
  at: number;
  url: string;
}

/** "detached" -> "attaching" -> "attached", or "lost" when the wire dies. */
export type Status = "detached" | "attaching" | "attached" | "lost";
