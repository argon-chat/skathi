export type NodeId = number;

export type Bounds = { x: number; y: number; w: number; h: number } | null;

/** A gpui window id in the target process. `windows` lists them. */
export type WindowId = number;

export interface Target {
  slot: string;
  dir: string;
  port: number;
  pid: number;
  /** The handshake token every command must carry. */
  token: string;
  alive: boolean;
  self: boolean;
}

export interface TargetList {
  preferred: string | null;
  self: { pid: number; dir: string } | null;
  targets: Target[];
  error?: string;
}

export interface DevtoolsArgs {
  target: string | null;
  window: WindowId | null;
  parent: number | null;
  embedded: boolean;
}

export interface PingReply {
  pid: number;
  [key: string]: unknown;
}

export interface TreeNode {
  id: NodeId;
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
  computed: Record<string, string[]>;
}

export interface Declaration {
  property: string;
  value: string;
  important?: boolean;
}

export interface Rule {
  selector: string;
  specificity: number;
  order: number;
  state: string;
  declarations: Declaration[];
  media?: string;
  ancestorState?: boolean;
  pseudo?: string;
  diagnostics?: string[];
}

export interface RulesReply {
  id: NodeId;
  count: number;
  rules: Rule[];
  note?: string;
}

export interface PickReply {
  x: number;
  y: number;
  /**
   * gpui's own `Window::is_window_hovered`. A synthesized pointer does not set it — only a real
   * cursor — which is what makes the latch reliable.
   */
  insideWindow: boolean;
  window: WindowId;
  viewport: { w: number; h: number };
  path: NodeId[];
  node: TreeNode | null;
}

export interface StyleRun {
  at: number;
  css: string;
}

export interface LogEntry {
  seq: number;
  /** Milliseconds since the target's log ring was enabled. */
  at: number;
  kind: string;
  text: string;
  styles?: StyleRun[] | null;
}

export interface LogsReply {
  entries: LogEntry[];
  /** The cursor to pass as `since` next time. */
  next: number;
  status: string | null;
  /** How many entries the ring dropped before we asked. */
  dropped?: number;
}

export interface EvalReply {
  value?: unknown;
  type?: string;
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
  /**
   * Present ONLY when the readback found transparent pixels — a mica/acrylic backdrop, which DWM
   * composites under the app where the capture cannot see it.
   */
  hasAlpha?: boolean;
  note?: string;
}

export interface WindowInfo {
  id: WindowId;
  kind: string;
  title: string;
  root: NodeId;
  viewport: { w: number; h: number };
  scale: number;
  focused: boolean;
  hovered: boolean;
}

export interface WindowsReply {
  windows: WindowInfo[];
  pid: number;
  inspectors: { window: WindowId | null; pid: number }[];
}

export interface PerfReply {
  frames: number;
  uptimeMs: number;
  /** `null` before the first frame happened. */
  bootMs: number | null;
  avgMs: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  fps: number;
  /** `null` when the platform does not report it (and absent from a target older than this field). */
  budgetMs?: number | null;
  /** Frame to frame, waiting included. */
  samples: number[];
  /** Per frame, beside `samples`: the frame requests' own time. Absent from an older target. */
  work?: number[];
  /** Per frame: 1 idle pacing chose the wait, 2 drew, 4 a request was held by the frame-rate limit. */
  flags?: number[];
  workP50Ms?: number;
  workP95Ms?: number;
  workMaxMs?: number;
  pacedShare?: number;
  drewShare?: number;
  throttledShare?: number;
  nodes: number;
  windows: number;
  phases: boolean;
}

export interface MemItem {
  label: string;
  bytes: number;
  info?: boolean;
}

export interface MemCategory {
  /**
   * `images` | `mapped` | `jsc` | `rust` | `other`, and `graphics` from a macOS host — stable, the
   * colour key.
   */
  key: string;
  label: string;
  bytes: number;
  note: string;
  items: MemItem[];
  residentBytes?: number;
  residentPrivateBytes?: number;
}

export interface MemBreakdown {
  basis: string;
  /**
   * Windows: the committed address space (private + mapped + image). macOS (`"footprint"`): the
   * physical footprint, Activity Monitor's "Memory".
   */
  totalLabel?: string;
  totalBytes: number;
  /**
   * Windows: the commit charge (Task Manager's "Commit size"). macOS: the kernel's physical
   * footprint.
   */
  privateBytes: number;
  workingSetBytes: number;
  /** Windows: Task Manager's "Memory" column. macOS: anonymous memory in RAM (`internal`). */
  privateWorkingSetBytes: number;
  rustCounting: boolean;
  categories: MemCategory[];
  gpu: { dedicatedBytes: number; sharedBytes: number; inBar?: boolean; items: MemItem[] };
}

export interface MemReply {
  breakdown: MemBreakdown;
  collectMs: number;
  gc?: { ms: number; freedPrivateBytes: number; freedJscHeapBytes: number };
}

export interface MemUnsupported {
  supported: false;
  reason: string;
}

export interface NetEntry {
  seq: number;
  rid?: number;
  /**
   * `"fetch"`, `"xhr"` or `"ws"`, or `"reload"`: a marker row the host put in where it ran the app
   * again in the same realm (`cause` says why).
   */
  kind: string;
  url: string;
  /** A reload marker only: why the host ran the app again, e.g. */
  cause?: string;
  method?: string;
  /** Milliseconds since the epoch, as the TARGET measured it. */
  at: number;
  ms?: number;
  status?: number;
  ok?: boolean;
  type?: string;
  error?: string;
  phase?: string;
  sent?: number;
  received?: number;
  proto?: string;
  conn?: number;
  reused?: boolean;
  cache?: "hit" | "revalidated" | "stored" | "miss";
}

export interface PoolStats {
  h1: number;
  h2: number;
  requests: number;
  reused: number;
  retries: number;
}

export interface NetReply {
  entries: NetEntry[];
  next: number;
  dropped: number;
  pool?: PoolStats;
}

export type HeaderPair = [string, string];

/**
 * `data` is absent when nothing was read (`note` says why) or the bytes were `evicted` to stay in
 * the budget.
 */
export interface NetBody {
  encoding?: "utf8" | "base64";
  data?: string;
  stored: number;
  truncated?: boolean;
  total?: number;
  /** How much the tee saw before it stopped reading. */
  seen?: number;
  complete?: boolean;
  unit?: string;
  mime?: string;
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

export interface NetDetail {
  entry: NetEntry;
  /** `null` when the host has no record for the row — `note` says why. */
  detail?: null;
  note?: string;
  rid?: number;
  seqs?: number[];
  kind?: string;
  url?: string;
  method?: string;
  at?: number;
  request?: {
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
    responseType?: string;
    body?: NetBody | null;
  };
  /** The cookie side — script never sees either header. */
  cookies?: {
    sent: string | null;
    set: string[];
    wire?: string[];
    pending?: boolean;
    note?: string;
  } | null;
  transport?: {
    protocol?: string;
    reused?: boolean;
    connectionId?: number;
    remote?: string | null;
    cache?: string;
  };
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

export type PushStream = "logs" | "perf" | "tree" | "net" | "status";

export interface Row {
  id: NodeId;
  depth: number;
  node: TreeNode;
  children: number;
  truncated: number;
  open: boolean;
  hit?: boolean;
}

export interface ConsoleLine {
  seq: number;
  at: number;
  kind: string;
  text: string;
  styles?: StyleRun[] | null;
  uid?: number;
}

export interface MenuItem {
  label: string;
  hint?: string | undefined;
  disabled?: boolean;
  divider?: boolean;
  run: () => void;
}

export interface Shot extends ScreenshotReply {
  at: number;
  url: string;
}

export type Status = "detached" | "attaching" | "attached" | "lost";

export type Tab = "elements" | "network" | "performance" | "storage" | "targets";
