/**
 * THE command table — the one place a vn-devtools command is declared.
 *
 * There are two front-ends onto the same wire (docs/DEVTOOLS.md): `ctl.ts`, the
 * dependency-free CLI, and `mcp.ts`, the MCP server. They used to carry a copy
 * of the table each, and the copies drifted exactly as copies do: milestone 21
 * added six commands (`windows`, `perf`, `net`, `storage`, `wheel`, `raise`)
 * and the optional `window` argument to the CLI, and the MCP server still
 * exposed the milestone-10 set.
 *
 * So neither file has a command list any more. This one owns:
 *
 *   * the wire `cmd` and its arguments, with types and prose;
 *   * how a CLI invocation reads into a request (`cli`) — those shapes are the
 *     CLI's own (positionals, `--flags`, `ctl type <sel> <words...>`), which is
 *     why they live in a function rather than in a schema;
 *   * the usage lines `ctl --help` prints;
 *   * the JSON Schema and the argument pass-through `mcp.ts` registers.
 *
 * The MCP tool name is DERIVED (`vn_<cmd>`), never written down, so it cannot
 * name a command that does not exist. `test/devtools-commands.test.ts` asserts
 * the set here equals the set docs/DEVTOOLS.md documents, and that neither
 * front-end declares a `cmd` of its own.
 *
 * NOT here: `--dist`, `--timeout`, `--json`, `--window`. The first three are
 * transport, and `window` rides on EVERY request (`mcpTools` injects it into
 * every schema; `ctl.ts` merges it into every request) rather than being
 * repeated per command.
 */

import { resolve } from "node:path";

// ---------------------------------------------------------------------------
// argv shapes
// ---------------------------------------------------------------------------

export type Flags = Record<string, string | boolean>;

/** A CLI invocation this table cannot read. `ctl.ts` prints it and exits 1. */
export class UsageError extends Error {}

// The explicit annotation is load-bearing: TypeScript only narrows after a
// `never`-returning call when the callee is a function declaration or a const
// with a declared type.
const die: (message: string) => never = (message) => {
  throw new UsageError(message);
};

/** `--index 2` -> 2; `--index` with nothing after it, or junk -> undefined. */
export const num = (v: unknown): number | undefined => {
  if (v === undefined || v === "") return undefined;
  const n = Number(v);
  return Number.isFinite(n) ? n : undefined;
};

/** Drops the `undefined`s, so the host never sees `"index": null`. */
const compact = (o: Record<string, unknown>): Record<string, unknown> => {
  for (const k of Object.keys(o)) if (o[k] === undefined) delete o[k];
  return o;
};

// ---------------------------------------------------------------------------
// the schema vocabulary
// ---------------------------------------------------------------------------

export interface ArgSpec {
  type: "string" | "number" | "boolean";
  description: string;
  required?: boolean;
}

const str = (description: string, required?: boolean): ArgSpec =>
  required ? { type: "string", description, required } : { type: "string", description };
const int = (description: string): ArgSpec => ({ type: "number", description });
const bool = (description: string): ArgSpec => ({ type: "boolean", description });

const SELECTOR = str(
  "a CSS selector, matched by the same vn-css cascade matcher the app styles with",
);
const SELECTOR_REQ = str(
  "a CSS selector, matched by the same vn-css cascade matcher the app styles with",
  true,
);
const INDEX = int("which match to use when the selector matches several (0-based)");
const NODE = int("a node id from vn_tree/vn_query, when you already have one");
const SINCE = int("first sequence number to return (the `next` from the last call)");

/**
 * The one argument every command takes, injected into every MCP schema by
 * [`mcpTools`] and merged into every CLI request by `ctl.ts`. See
 * docs/DEVTOOLS.md, "The `window` argument".
 */
export const WINDOW_ARG = int(
  "which of the target's OS windows the command is about (vn_windows lists them; " +
    "default: the primary one). One process can paint several windows off one retained " +
    "tree, and they are in different client spaces",
);

export interface CommandSpec {
  /** The wire `cmd`. The MCP tool is `vn_<cmd>` and the CLI verb is this. */
  cmd: string;
  /**
   * Other CLI spellings of the same verb. The request still carries `cmd`, so
   * an alias never reaches the wire and never becomes a second MCP tool.
   */
  aliases?: string[];
  /** The MCP tool description. */
  description: string;
  /** `ctl --help` lines for this command, already column-aligned. */
  usage: string[];
  /** The wire arguments, in the order the MCP schema lists them. */
  args: Record<string, ArgSpec>;
  /** Reads a CLI invocation (positionals after the verb, flags) into a request. */
  cli: (rest: string[], flags: Flags) => Record<string, unknown>;
}

/** `--index N` folded onto a request, when it was given. */
const withIndex = (o: Record<string, unknown>, flags: Flags): Record<string, unknown> => {
  const index = num(flags.index);
  return index === undefined ? o : { ...o, index };
};

/** The `selector | --node ID` pair `styles`, `rules` and `highlight` share. */
const selectorOrNode = (cmd: string, rest: string[], flags: Flags): Record<string, unknown> => {
  const node = num(flags.node);
  if (!rest[0] && node === undefined) die(`${cmd} needs a selector or --node <id>`);
  return compact({ selector: rest[0] || undefined, node });
};

// ---------------------------------------------------------------------------
// the table
// ---------------------------------------------------------------------------

export const COMMANDS: CommandSpec[] = [
  {
    cmd: "ping",
    description: "Is a host alive on this dist slot? Returns {pid, root}.",
    usage: ["  ping                                is a host alive on this dist slot?"],
    args: {},
    cli: () => ({}),
  },
  {
    cmd: "query",
    description:
      "querySelectorAll through the REAL cascade matcher. Returns [{id, tag, classes, text, " +
      "bounds, scopes}]. An unsupported selector (`:has()`, `::before`, `:visited`, ...) is an " +
      "error carrying the same advisory the stylesheet compiler prints.",
    usage: ["  query    <selector>                 match through the real vn-css cascade matcher"],
    args: { selector: SELECTOR_REQ },
    cli: (rest) => {
      if (!rest[0]) die("query needs a selector");
      return { selector: rest[0] };
    },
  },
  {
    cmd: "text",
    description: "textContent of the matched subtree (or a native <input>'s live buffer).",
    usage: [
      "  text     <selector> [--index N]     textContent of the subtree (or an <input>'s buffer)",
    ],
    args: { selector: SELECTOR_REQ, index: INDEX },
    cli: (rest, flags) => {
      if (!rest[0]) die("text needs a selector");
      return withIndex({ selector: rest[0] }, flags);
    },
  },
  {
    cmd: "tree",
    description:
      "Dump the retained tree (kind / tag / classes / props / scopes / listeners / text / " +
      "bounds / children). `kind` is element | text | anchor — a text node and a Vue fragment " +
      "anchor both have an EMPTY tag, so label a row by kind, never by tag.",
    usage: ["  tree     [selector] [--depth N] [--node ID]   dump the retained tree"],
    args: {
      selector: str("subtree root (default: the mounted root, or what `window` paints)"),
      node: int("subtree root as a node id, when you already have one"),
      depth: int("levels to descend (default 6)"),
    },
    cli: (rest, flags) =>
      compact({ selector: rest[0] || undefined, node: num(flags.node), depth: num(flags.depth) }),
  },
  {
    cmd: "styles",
    description:
      "The node's folded ComputedStates: base + the hover/active/focus/focus-visible rungs, " +
      "transitions and extras, plus `live` (the style in EFFECT this frame). Takes a node id " +
      "instead of a selector when you already have one.",
    usage: ["  styles   <selector|--node ID> [--index N]     the node's folded ComputedStates"],
    args: { selector: SELECTOR, index: INDEX, node: NODE },
    cli: (rest, flags) => withIndex(selectorOrNode("styles", rest, flags), flags),
  },
  {
    cmd: "rules",
    description:
      "WHICH rules produced this node's styles, in cascade order ((specificity, order) ascending, " +
      "so the last declaration of a property wins). Each rule carries its selector, @media " +
      "prelude, state bucket (base/hover/active/focus/...), pseudo-element, declarations as the " +
      "author wrote them, and the vn-css advisories about THAT rule — i.e. which of its " +
      "declarations this engine does not implement.",
    usage: [
      "  rules    <selector|--node ID> [--index N]     the RULES that matched it, in",
      "                                     cascade order — selector, @media, source",
      "                                     order, declarations, per-rule advisories",
    ],
    args: { selector: SELECTOR, index: INDEX, node: NODE },
    cli: (rest, flags) => withIndex(selectorOrNode("rules", rest, flags), flags),
  },
  {
    cmd: "highlight",
    description:
      "Draw the inspector's outline over a node in the app (a blue box plus a `tag.class · WxH` " +
      "chip), or clear it. Painted by the host shell, so it never appears in the app's own tree " +
      "and never takes a click. Pass ttlMs to make it expire by itself; `window` decides which " +
      "window paints it.",
    usage: [
      "  highlight <selector|--node ID> [--index N] [--ttl MS] [--label TEXT]",
      "  highlight --clear                  the inspector outline, drawn over the app",
    ],
    args: {
      selector: SELECTOR,
      index: INDEX,
      node: NODE,
      ttlMs: int("expire after this many ms (default: until cleared)"),
      label: str("chip text (default: tag.class · WxH)"),
      clear: bool("remove the outline"),
    },
    cli: (rest, flags) => {
      const node = num(flags.node);
      if (flags.clear || (!rest[0] && node === undefined)) return { clear: true };
      return withIndex(
        compact({
          selector: rest[0] || undefined,
          node,
          ttlMs: num(flags.ttl),
          label: typeof flags.label === "string" && flags.label ? flags.label : undefined,
        }),
        flags,
      );
    },
  },
  {
    cmd: "pick",
    description:
      "Where the pointer is over the app and which node is under it: {x, y, insideWindow, " +
      "viewport, path[], node}. The hit test is over the painted bounds (deepest rectangle wins), " +
      "not gpui's hitboxes. `insideWindow` is gpui's window hover flag — a synthesized pointer " +
      "event does not set it, only a real cursor does. WITHOUT `window` every window is asked " +
      "and the one the pointer is actually in answers; the reply always names it.",
    usage: [
      "  pick                               where the pointer is over the app and which",
      "                                     node is under it (the picker's poll)",
    ],
    args: {},
    cli: () => ({}),
  },
  {
    cmd: "click",
    description:
      "Synthesize mousedown -> mouseup -> click at the node's centre, through the app's own " +
      "dispatch (capture/bubble, preventDefault honored). NOT a real pointer: :hover / :active " +
      "visuals are not exercised and nothing proves the node is reachable — use vn_mouse for " +
      "that. Errors on 0 or >1 matches unless `index` is given.",
    usage: ["  click    <selector> [--index N] [--button N]"],
    args: { selector: SELECTOR_REQ, index: INDEX, button: int("0 left, 1 middle, 2 right") },
    cli: (rest, flags) => {
      if (!rest[0]) die("click needs a selector");
      return withIndex(compact({ selector: rest[0], button: num(flags.button) }), flags);
    },
  },
  {
    cmd: "focus",
    description:
      "el.focus(). The focus/blur events come from gpui's focus observers, so it reports " +
      "identically to a click or a Tab.",
    usage: ["  focus    <selector> [--index N]"],
    args: { selector: SELECTOR_REQ, index: INDEX },
    cli: (rest, flags) => {
      if (!rest[0]) die("focus needs a selector");
      return withIndex({ selector: rest[0] }, flags);
    },
  },
  {
    cmd: "type",
    description:
      "Focus the node and type `text` one keystroke at a time through gpui's real key path, so " +
      "keydown / input / v-model all fire exactly as they do for a physical key — including the " +
      "app's own preventDefault() guards.",
    usage: ["  type     <selector> <text> [--index N]"],
    args: { selector: SELECTOR_REQ, text: str("the text to type", true), index: INDEX },
    cli: (rest, flags) => {
      if (rest.length < 2) die('type needs a selector and text: ctl type ".field" "hello"');
      return withIndex({ selector: rest[0], text: rest.slice(1).join(" ") }, flags);
    },
  },
  {
    cmd: "ime",
    description:
      "Stand in for the IME on a text field: `text` becomes the composition's pre-edit, or with " +
      "`commit` its committed text (\"\" cancels it). compositionstart, beforeinput, " +
      "compositionupdate, input and compositionend fire in Chromium's order, from the widget's " +
      "own path. For testing where no IME is installed; no keystroke is sent.",
    usage: ["  ime      <selector> <text> [--commit] [--index N]"],
    args: {
      selector: SELECTOR_REQ,
      text: str('the pre-edit, or the committed text; "" cancels the composition', true),
      commit: bool("commit the composition with `text`"),
      index: INDEX,
    },
    cli: (rest, flags) => {
      if (!rest[0]) die('ime needs a selector and text: ctl ime ".field" "か" [--commit]');
      return withIndex(compact({ selector: rest[0], text: rest.slice(1).join(" "), commit: flags.commit === true || undefined }), flags);
    },
  },
  {
    cmd: "key",
    description:
      "Send one keystroke: `Enter`, `Escape`, `Shift+Tab`, `Ctrl+A`, `ArrowDown`, ... With a " +
      "selector the node is focused first; without one it goes to whatever already has focus.",
    usage: ["  key      <Key|Mod+Key> [selector] [--index N]"],
    args: { key: str("key name, optionally Mod+Key", true), selector: SELECTOR, index: INDEX },
    cli: (rest, flags) => {
      if (!rest[0]) die("key needs a key name: ctl key Enter [selector]");
      // With no selector the keystroke goes to whatever already has focus.
      return withIndex(compact({ key: rest[0], selector: rest[1] || undefined }), flags);
    },
  },
  {
    cmd: "mouse",
    description:
      "A REAL pointer event at a window coordinate, through gpui's own hit test — so z-order, " +
      "content masks, `pointer-events` and the hover ladder all decide what happens. This is what " +
      "proves an element is reachable (and the only way to exercise :hover); vn_click proves the " +
      "handler chain instead. Aim it with a selector (the node's centre) or with x + y. Reports " +
      "which node holds focus afterwards. No cursor moves and no window is activated.",
    usage: [
      "  mouse    [selector] [--x N --y N] [--kind move|down|up|click] [--button N]",
      "           [--held] [--clicks N] [--shift]",
      "                                     a REAL pointer event through gpui's hit",
      "                                     test (hover, z-order, pointer-events all",
      "                                     apply); reports which node has focus after",
    ],
    args: {
      selector: SELECTOR,
      index: INDEX,
      x: int("window x, when no selector is given"),
      y: int("window y, when no selector is given"),
      kind: str("move (default) | down | up | click"),
      button: int("0 left, 1 middle, 2 right"),
      held: bool("the move is a drag: the button is down during it (a text selection follows it)"),
      clicks: int("the press's click count: 2 selects a word, 3 a paragraph"),
      shift: bool("Shift is held (Shift+click extends a text selection)"),
    },
    cli: (rest, flags) => {
      // Unlike `click`, which synthesizes into the app's dispatcher from a node
      // id, this one is decided by the hit test — so it is what proves an
      // element is actually reachable, and it is how `:hover` is exercised.
      const base = compact({
        kind: flags.kind ? String(flags.kind) : "move",
        button: num(flags.button),
        held: flags.held ? true : undefined,
        clicks: num(flags.clicks),
        shift: flags.shift ? true : undefined,
      });
      if (rest[0]) return withIndex({ ...base, selector: rest[0] }, flags);
      const x = num(flags.x);
      const y = num(flags.y);
      if (x === undefined || y === undefined) die("mouse needs a selector, or both --x and --y");
      return { ...base, x, y };
    },
  },
  {
    cmd: "scroll",
    description:
      "Scroll a container by (dx, dy) CSS pixels through its own gpui ScrollHandle and fire the " +
      "`scroll` event. The node must have `overflow: scroll|auto` in the cascade. This proves a " +
      "container CAN scroll and says nothing about the gesture — vn_wheel is the gesture.",
    usage: ["  scroll   <selector> <dy> [--dx N] [--index N]"],
    args: {
      selector: SELECTOR_REQ,
      dy: int("vertical delta, positive = down"),
      dx: int("horizontal delta"),
      index: INDEX,
    },
    cli: (rest, flags) => {
      if (!rest[0]) die("scroll needs a selector");
      return withIndex(
        { selector: rest[0], dx: num(flags.dx) ?? 0, dy: num(rest[1]) ?? num(flags.dy) ?? 0 },
        flags,
      );
    },
  },
  {
    cmd: "wheel",
    description:
      "A REAL wheel event at a window coordinate, through gpui's own hit test — what an app's own " +
      "@wheel handler sees. THE DELTAS ARE THE DOM'S: dy > 0 scrolls DOWN, as WheelEvent.deltaY " +
      "means (gpui carries the platform's opposite sign; the conversion is one place on each " +
      "side). Aim it with a selector or with x + y.",
    usage: [
      "  wheel    [selector] [--x N --y N] --dy N [--dx N]",
      "                                     a REAL wheel event through gpui's hit",
      "                                     test — what an app's own @wheel handler",
      "                                     sees. DOM signs: --dy 200 scrolls DOWN.",
      "                                     (`scroll` drives a container's handle",
      "                                     directly and never involves the wheel)",
    ],
    args: {
      selector: SELECTOR,
      index: INDEX,
      x: int("window x, when no selector is given"),
      y: int("window y, when no selector is given"),
      dy: int("vertical delta, DOM signs: positive scrolls DOWN"),
      dx: int("horizontal delta"),
    },
    cli: (rest, flags) => {
      const dy = num(flags.dy) ?? 0;
      const dx = num(flags.dx) ?? 0;
      if (!dy && !dx) die("wheel needs --dy (or --dx); DOM signs, so --dy 200 scrolls down");
      if (rest[0]) return withIndex({ selector: rest[0], dx, dy }, flags);
      const x = num(flags.x);
      const y = num(flags.y);
      if (x === undefined || y === undefined) die("wheel needs a selector, or both --x and --y");
      return { dx, dy, x, y };
    },
  },
  {
    cmd: "eval",
    description:
      "Evaluate JavaScript inside the app's JSC, in global scope. Returns {value, type} or " +
      "{exception, stack}. The escape hatch for everything the other tools do not cover.",
    usage: ["  eval     <js source>"],
    args: { js: str("JavaScript source", true) },
    cli: (rest) => {
      if (!rest.length) die("eval needs a JS source string");
      return { js: rest.join(" ") };
    },
  },
  {
    cmd: "logs",
    description:
      "The host's log ring (500 entries): console output, [js uncaught] errors, CSS diagnostics " +
      "and compat advisories, plus `next` (the cursor) and the current status line. An entry may " +
      "carry `styles` — where each `%c` run began and what it declared. Page with `since`.",
    usage: ["  logs     [--since N]"],
    args: { since: SINCE },
    cli: (_rest, flags) => compact({ since: num(flags.since) }),
  },
  {
    cmd: "screenshot",
    description:
      "Capture the app window to a PNG. Default backend is gpui's render_to_image readback: the " +
      "CLIENT area exactly (clientX/clientY come back 0), works unfocused, occluded, off-screen " +
      "and minimized, and never raises or activates the window — but it cannot see a mica/acrylic " +
      "backdrop, which DWM composites under the app. Pass backend=print_window for the composed " +
      "image (PrintWindow(PW_RENDERFULLCONTENT), whole window rect, stale when minimized).",
    usage: ["  screenshot [path] [--backend auto|render_to_image|print_window]"],
    args: {
      path: str("output PNG path (default: <dist>/screenshots/<n>.png)"),
      backend: str('"auto" (default), "render_to_image" or "print_window"'),
    },
    cli: (rest, flags) =>
      compact({
        // A relative path on a CLI means "relative to where I typed it", and
        // the host's cwd is its own slot.
        path: rest[0] ? resolve(rest[0]) : undefined,
        backend: flags.backend ? String(flags.backend) : undefined,
      }),
  },
  {
    cmd: "windows",
    description:
      "Every OS window this process paints — {id, kind (main|popout), title, root, viewport, " +
      "scale, focused, hovered} — plus `inspectors`, the devtools windows the target has open on " +
      "ITSELF via F12. One process can paint several windows off one retained tree, so this is " +
      "where the `window` argument's values come from.",
    usage: [
      "  windows                            every OS window this process paints (and",
      "                                     every inspector it has open on itself)",
    ],
    args: {},
    cli: () => ({}),
  },
  {
    cmd: "perf",
    description:
      "The rolling frame window, always on: {frames, uptimeMs, bootMs, avgMs, p50Ms, p95Ms, " +
      "maxMs, fps, budgetMs, samples[], nodes, windows, phases}. `budgetMs` is the display's " +
      "refresh interval (null when unreported). One Instant read and one store per frame, " +
      "so \"why is this janky\" can be asked of a host that is already running. `phases` reports " +
      "whether VN_PERF's stderr breakdown is on rather than duplicating its counters (reading " +
      "them would clear them under the person profiling).",
    usage: [
      "  perf                               the rolling frame window: fps, p50/p95,",
      "                                     node count, boot->first-frame",
    ],
    args: {},
    cli: () => ({}),
  },
  {
    cmd: "mem",
    description:
      "Where the process's memory is, computed on demand. `breakdown` (BYTES) has categories that " +
      "SUM to the committed address space — loaded images (exe + DLLs), mapped files, JSC " +
      "(mimalloc + JIT), the Rust heap (the process heap, split by subsystem when counted), and " +
      "'other private' (driver heaps, stacks: what no allocator accounts for) — plus GPU memory " +
      "beside them. The other sections (MB; `…Bytes` fields are bytes): OS counters, the " +
      "address-space walk, heap summaries, the Rust heap count (on by default in a dev build, " +
      "VN_MEMSTATS=0/1 overrides), JSC's GC heap and mimalloc, GPU per adapter LUID, gpui's sprite " +
      "atlas and decoded-image cache, the painted-image registry, the retained tree per node, the " +
      "devtools' own rings. `gc` first runs two full synchronous JSC collections and returns free " +
      "pages to the OS (it changes state) and reports what that freed; `code` does the same after deleting " +
      "all compiled code that is not running (it recompiles on demand), which tells code from data.",
    usage: [
      "  mem      [--gc] [--code] [--regions]  where the process's memory is (--gc: a full JSC",
      "                                     collection first, before/after reported;",
      "                                     --code: also delete compiled code first;",
      "                                     --regions: walk the heap too, list the biggest",
      "                                     unattributed reservations)",
    ],
    args: {
      gc: bool("run two full JSC collections first and report before/after (changes state)"),
      code: bool("like gc, after deleting all compiled code that is not running (bytecode, JIT): what is left is data"),
      regions: bool(
        "also walk the process heap (it is locked meanwhile — tens of ms on a big heap) so the " +
          "address walk can tell the heap's reservations apart, and list the biggest unattributed ones",
      ),
    },
    cli: (_rest, flags) =>
      compact({ gc: flags.gc ? true : undefined, code: flags.code ? true : undefined, regions: flags.regions ? true : undefined }),
  },
  {
    cmd: "trace",
    description:
      "A frame-by-frame record of where the host's time went during an interaction. `start` arms " +
      "every phase counter; `stop` disarms and returns the summary (and, with `frames`, every " +
      "non-idle frame: pump / JS / DOM ops and the span between the first and last one / layout " +
      "and style reads / FORCED synchronous layouts and their callers / restyles, whole-document " +
      "ones and why / sheet recompiles / build / gpui layout-request, taffy, prepaint, paint / " +
      "text shaping misses / image resample copies / web fonts registered). `peek` reports " +
      "without stopping; `mark` labels the frame in progress. Refused while VN_PERF is on.",
    usage: [
      "  trace    start [--cap N] | stop [--frames] | peek [--frames] | mark <label>",
      "                                     a frame-by-frame record of an interaction",
    ],
    args: {
      op: str('"start", "stop", "peek" (default) or "mark"'),
      frames: bool("include every stored frame, not only the summary"),
      label: str("for op=mark: the label"),
      cap: int("for op=start: the most active frames kept (default 20000)"),
    },
    cli: (rest, flags) => {
      const op = rest[0] || "peek";
      if (op === "mark" && !rest[1] && typeof flags.label !== "string") die("trace mark needs a label");
      return compact({
        op,
        frames: flags.frames ? true : undefined,
        label: op === "mark" ? (typeof flags.label === "string" ? flags.label : rest.slice(1).join(" ")) : undefined,
        cap: num(flags.cap),
      });
    },
  },
  {
    cmd: "net",
    description:
      "Every fetch(), XMLHttpRequest and WebSocket the app opened, from BOOT — a bounded ring of " +
      "300 summary rows. `kind` is fetch, xhr or ws; a socket's `phase` is open-requested / open / " +
      "error / close and its messages are COUNTED rather than logged. The ring outlives " +
      "location.reload() and a dev host's cold restart: the host puts a `kind: reload` marker row " +
      "(`cause` says which) between " +
      "the runs, and the rows before it are the previous run's. " +
      "Page with `since`. A row " +
      "that came off the network carries `proto` (h2 / http/1.1), `conn` (connection id; equal ids " +
      "shared one) and `reused`; `pool` is the transport's connection/request/reuse/retry counters. " +
      "Headers, bodies, cookies and timing are NOT here — pass a row's `seq` to vn_net_detail.",
    usage: ["  net      [--since N]               the fetch / WebSocket / XHR log"],
    args: { since: SINCE },
    cli: (_rest, flags) => compact({ since: num(flags.since) }),
  },
  {
    cmd: "net_detail",
    aliases: ["net-detail"],
    description:
      "Everything behind ONE vn_net row, asked for by its `seq`: the request (URL, method, the " +
      "headers the app passed, credentials, the body — text, form fields, or bytes) and the " +
      "response (status, headers, a body preview). A body is `encoding: utf8` text or `base64` " +
      "bytes (binary types such as application/ion are always base64), capped at 256 KB with " +
      "`truncated` set; the most recent ~12 MB of bodies are kept and an older one says " +
      "`evicted`. `cookies.sent` is the Cookie header the jar sent; `cookies.wire` the response's " +
      "Set-Cookie lines exactly as received (`cookies.set`, reconstructed from the jar, when there " +
      "is no wire copy) — script never sees either. `transport` has protocol, reused, " +
      "connectionId and the remote address. " +
      "`timing` has headersMs / bodyMs / failedMs; a WebSocket row answers with its socket: " +
      "protocol, close code and reason, and sent/received message counts.",
    usage: [
      "  net_detail <seq>                   headers, bodies, cookies and timing behind one",
      "                                     `net` row (also spelled net-detail)",
    ],
    args: {
      seq: { type: "number", description: "the row's `seq`, from vn_net", required: true },
    },
    cli: (rest, flags) => {
      const seq = num(rest[0]) ?? num(flags.seq);
      if (seq === undefined) die("net_detail needs a row's seq (from `net`): ctl net_detail 12");
      return { seq };
    },
  },
  {
    cmd: "storage",
    description:
      "localStorage / sessionStorage, read and written through the APP's own JS rather than the " +
      "host's disk layer — so what you see is what getItem() answers, pending writes included. " +
      "A host built without the `storages` feature answers with the error that says so, rather " +
      "than an empty table.",
    usage: ["  storage  [local|session] [list|set|remove|clear] [--key K] [--value V]"],
    args: {
      area: str('"local" (default) or "session"'),
      op: str('"list" (default), "set", "remove" or "clear"'),
      key: str("the entry to set or remove"),
      value: str("the value, for op=set"),
    },
    cli: (rest, flags) =>
      compact({
        area: rest[0] || "local",
        op: rest[1] || "list",
        key: typeof flags.key === "string" ? flags.key : undefined,
        value: typeof flags.value === "string" ? flags.value : undefined,
      }),
  },
  {
    cmd: "dialog",
    description:
      "The dialogs waiting for an answer — <input type=file>, the File System Access pickers, " +
      "<a download>'s Save As, and the app's permission prompt — when the app runs with " +
      "VN_DIALOGS=scripted (nothing reaches the OS then). `list` names each one ({id, window, " +
      "what, kind, ...}: a file dialog's multiple, filters, startIn, suggestedName; a question's " +
      "(kind \"ask\") instruction and buttons); `answer` finishes a file dialog with absolute " +
      "paths, checked as the OS dialog would check them; `choose` presses a question's button " +
      "(its label, e.g. \"Allow\", or its index); `cancel` dismisses either, as Esc would. " +
      "Without VN_DIALOGS=scripted every dialog is real and this command refuses.",
    usage: [
      "  dialog   [list]                     dialogs waiting for an answer (VN_DIALOGS=scripted)",
      "  dialog   answer <id> <path>...      answer a file dialog with absolute paths",
      "  dialog   choose <id> <button>       press a question's button (label or index)",
      "  dialog   cancel <id>                dismiss one, as Esc would",
    ],
    args: {
      op: str('"list" (default), "answer", "choose" or "cancel"'),
      id: int("the dialog to answer, choose in or cancel (from `list`)"),
      paths: str("for op=answer: absolute paths, separated by `|` (the wire also takes an array)"),
      button: str('for op=choose: the button\'s label ("Allow", "Block") or its index'),
    },
    cli: (rest, flags) => {
      const op = rest[0] || "list";
      // `dialog answer 3 C:\a.txt` or `dialog answer --id 3 C:\a.txt`
      const flagged = num(flags.id);
      const id = flagged ?? num(rest[1]);
      if (op !== "list" && id === undefined) die(`dialog ${op} needs the dialog's id`);
      const listed = rest.slice(flagged === undefined ? 2 : 1);
      if (op === "choose") {
        const button = typeof flags.button === "string" ? flags.button : listed.join(" ");
        if (!button) die("dialog choose needs a button: its label or its index");
        return compact({ op, id, button });
      }
      const paths = typeof flags.paths === "string" ? flags.paths : listed.length ? listed.join("|") : undefined;
      if (op === "answer" && paths === undefined) die("dialog answer needs at least one path");
      return compact({ op, id, paths });
    },
  },
  {
    cmd: "permissions",
    description:
      "The app's permissions: every capability's state (granted / denied / prompt / unsupported), " +
      "the answers stored for the app's own prompt (microphone: allow / block, and when) and the " +
      "file they live in (permissions.json in the app's storage folder). `reset` forgets a stored " +
      "answer, so the next getUserMedia shows the prompt again. That and the app's own " +
      "appPermissions.request(\"microphone\") are the ways back after Block. It changes a real " +
      "decision on any host, scripted or not.",
    usage: [
      "  permissions [list]                  states, stored answers, where they are kept",
      "  permissions reset [name]            forget an answer (all of them without a name)",
    ],
    args: {
      op: str('"list" (default) or "reset"'),
      name: str('for op=reset: the permission, e.g. "microphone" (default: every one with a prompt)'),
    },
    cli: (rest, flags) =>
      compact({
        op: rest[0] || "list",
        name: typeof flags.name === "string" ? flags.name : rest[1],
      }),
  },
  {
    cmd: "raise",
    description:
      "Bring the addressed window to the front. The ONLY command here that deliberately takes " +
      "focus — VN_DEVTOOLS_RAISE=off in the target's environment suppresses the activation, for " +
      "an automated run on a machine somebody is using.",
    usage: ["  raise                              bring the window to the front"],
    args: {},
    cli: () => ({}),
  },
];

// ---------------------------------------------------------------------------
// front-end 1: MCP
// ---------------------------------------------------------------------------

export interface McpTool {
  name: string;
  description: string;
  inputSchema: {
    type: "object";
    properties: Record<string, { type: string; description: string }>;
    required?: string[];
  };
  /** Tool arguments -> the wire request, `cmd` included. */
  build: (args: Record<string, unknown>) => Record<string, unknown>;
}

/** `vn_query` for `query`. Derived, so a tool can never outlive its command. */
export const toolName = (cmd: string): string => `vn_${cmd}`;

/**
 * The MCP tool list, generated. Every tool takes `window`, exactly as every
 * wire command does — see [`WINDOW_ARG`].
 */
export function mcpTools(): McpTool[] {
  return COMMANDS.map((spec) => {
    const names = Object.keys(spec.args);
    const required = names.filter((n) => spec.args[n].required);
    const properties: Record<string, { type: string; description: string }> = {};
    for (const n of names) {
      properties[n] = { type: spec.args[n].type, description: spec.args[n].description };
    }
    properties.window = { type: WINDOW_ARG.type, description: WINDOW_ARG.description };
    return {
      name: toolName(spec.cmd),
      description: spec.description,
      inputSchema: {
        type: "object" as const,
        properties,
        ...(required.length ? { required } : {}),
      },
      build: (args: Record<string, unknown>) => {
        const wire: Record<string, unknown> = { cmd: spec.cmd };
        for (const n of [...names, "window"]) if (args[n] !== undefined) wire[n] = args[n];
        return wire;
      },
    };
  });
}

// ---------------------------------------------------------------------------
// front-end 2: the CLI
// ---------------------------------------------------------------------------

/** Every verb `ctl.ts` accepts, in usage order. */
export const commandNames = (): string[] => COMMANDS.map((c) => c.cmd);

/** The `commands` block of `ctl --help`. */
export function usageBlock(): string {
  return COMMANDS.flatMap((c) => c.usage).join("\n");
}

/**
 * `["query", ".tab"]` + flags -> `{cmd: "query", selector: ".tab"}`.
 * Throws [`UsageError`] for an unknown verb or a malformed invocation.
 */
export function buildRequest(argv: string[], flags: Flags): Record<string, unknown> {
  const [cmd, ...rest] = argv;
  const spec = COMMANDS.find((c) => c.cmd === cmd || (c.aliases ?? []).includes(cmd));
  if (!spec) die(`unknown command \`${cmd}\``);
  return { cmd: spec.cmd, ...spec.cli(rest, flags) };
}
