/**
 * The command table: the one place a sk-devtools command is declared, shared by `ctl.ts` (the
 * dependency-free CLI) and `mcp.ts` (the MCP server). It owns the wire `cmd` and its arguments, how
 * a CLI invocation reads into a request, the `ctl --help` usage lines, and the MCP JSON Schema.
 * The MCP tool name is derived (`sk_<cmd>`); `window` rides on every request.
 */

import { resolve } from "node:path";

export type Flags = Record<string, string | boolean>;

export class UsageError extends Error {}

// The explicit annotation is load-bearing: TypeScript narrows after a `never`-returning call only
// for a function declaration or a const with a declared type.
const die: (message: string) => never = (message) => {
  throw new UsageError(message);
};

export const num = (v: unknown): number | undefined => {
  if (v === undefined || v === "") return undefined;
  const n = Number(v);
  return Number.isFinite(n) ? n : undefined;
};

const compact = (o: Record<string, unknown>): Record<string, unknown> => {
  for (const k of Object.keys(o)) if (o[k] === undefined) delete o[k];
  return o;
};

const integerFlag = (
  value: unknown,
  name: string,
  minimum = 0,
  maximum = Number.MAX_SAFE_INTEGER,
): number | undefined => {
  if (value === undefined) return undefined;
  if (value === true || value === false || value === "") die(`${name} needs an integer`);
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum)
    die(`${name} must be an integer between ${minimum} and ${maximum}`);
  return parsed;
};
const stringFlag = (value: unknown): string | undefined => (typeof value === "string" ? value : undefined);

export interface ArgSpec {
  type: "string" | "number" | "boolean";
  description: string;
  required?: boolean;
}

const str = (description: string, required?: boolean): ArgSpec =>
  required ? { type: "string", description, required } : { type: "string", description };
const int = (description: string): ArgSpec => ({ type: "number", description });
const bool = (description: string): ArgSpec => ({ type: "boolean", description });

const SELECTOR = str("a CSS selector, matched by the same sk-css cascade matcher the app styles with");
const SELECTOR_REQ = str("a CSS selector, matched by the same sk-css cascade matcher the app styles with", true);
const INDEX = int("which match to use when the selector matches several (0-based)");
const NODE = int("a node id from sk_tree/sk_query, when you already have one");
const SINCE = int("first sequence number to return (the `next` from the last call)");

export const WINDOW_ARG = int(
  "which of the target's OS windows the command is about (sk_windows lists them; " +
    "default: the primary one). One process can paint several windows off one retained " +
    "tree, and they are in different client spaces",
);

export interface CommandSpec {
  cmd: string;
  /** Other CLI spellings of the verb; the request still carries `cmd`. */
  aliases?: string[];
  description: string;
  usage: string[];
  args: Record<string, ArgSpec>;
  cli: (rest: string[], flags: Flags) => Record<string, unknown>;
}

const withIndex = (o: Record<string, unknown>, flags: Flags): Record<string, unknown> => {
  const index = num(flags.index);
  return index === undefined ? o : { ...o, index };
};

const selectorOrNode = (cmd: string, rest: string[], flags: Flags): Record<string, unknown> => {
  const node = num(flags.node);
  if (!rest[0] && node === undefined) die(`${cmd} needs a selector or --node <id>`);
  return compact({ selector: rest[0] || undefined, node });
};

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
    usage: ["  query    <selector>                 match through the real sk-css cascade matcher"],
    args: { selector: SELECTOR_REQ },
    cli: (rest) => {
      if (!rest[0]) die("query needs a selector");
      return { selector: rest[0] };
    },
  },
  {
    cmd: "text",
    description: "textContent of the matched subtree (or a native <input>'s live buffer).",
    usage: ["  text     <selector> [--index N]     textContent of the subtree (or an <input>'s buffer)"],
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
    cli: (rest, flags) => compact({ selector: rest[0] || undefined, node: num(flags.node), depth: num(flags.depth) }),
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
      "author wrote them, and the sk-css advisories about THAT rule — i.e. which of its " +
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
    aliases: ["picker"],
    description:
      "Without op, poll the node under the real pointer: {x,y,insideWindow,window,path,node}. " +
      "With op=start, arm the native element picker; the next target click is consumed before " +
      "application handlers and selects a node. op=status renews its 15-second lease and returns " +
      "active/selected/cancelled, generation and selection. Escape or op=stop cancels.",
    usage: ["  pick     [start|status|stop]         poll, or control the native click picker"],
    args: { op: str('"start", "status" or "stop"; absent preserves stateless pointer polling') },
    cli: (rest, flags) => {
      const op = rest[0] ?? stringFlag(flags.op);
      if (op !== undefined && !["start", "status", "stop"].includes(op)) die("pick op must be start, status or stop");
      return compact({ op });
    },
  },
  {
    cmd: "click",
    description:
      "Synthesize mousedown -> mouseup -> click at the node's centre, through the app's own " +
      "dispatch (capture/bubble, preventDefault honored). NOT a real pointer: :hover / :active " +
      "visuals are not exercised and nothing proves the node is reachable — use sk_mouse for " +
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
      '`commit` its committed text ("" cancels it). compositionstart, beforeinput, ' +
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
      return withIndex(
        compact({ selector: rest[0], text: rest.slice(1).join(" "), commit: flags.commit === true || undefined }),
        flags,
      );
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
      return withIndex(compact({ key: rest[0], selector: rest[1] || undefined }), flags);
    },
  },
  {
    cmd: "mouse",
    description:
      "A REAL pointer event at a window coordinate, through gpui's own hit test — so z-order, " +
      "content masks, `pointer-events` and the hover ladder all decide what happens. This is what " +
      "proves an element is reachable (and the only way to exercise :hover); sk_click proves the " +
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
      // Unlike `click`, this is decided by the hit test, so it proves an element is reachable and exercises `:hover`.
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
    cmd: "pen",
    description:
      "A stylus event at a window coordinate, dispatched the way a platform reports a pen: the page gets " +
      '`pointerType: "pen"` pointer events with pressure and tilt, then the mouse events. A move with ' +
      "--from-x/--from-y and --steps N carries N readings --interval ms apart (default 4), its " +
      "getCoalescedEvents(), and feeds its getPredictedEvents(). No OS input.",
    usage: [
      "  pen      [selector] [--x N --y N] [--kind hover|down|move|up|leave] [--pressure P]",
      "           [--tilt-x D] [--tilt-y D] [--twist D] [--eraser] [--barrel]",
      "           [--from-x N --from-y N --from-pressure P --steps N --interval MS]",
      "                                     a pen event through gpui's dispatch",
    ],
    args: {
      selector: SELECTOR,
      index: INDEX,
      x: int("window x, when no selector is given"),
      y: int("window y, when no selector is given"),
      kind: str("hover (default) | down | move | up | leave"),
      pressure: int("tip pressure 0..1 while touching (default 0.5)"),
      tiltX: int("tilt toward +x, degrees"),
      tiltY: int("tilt toward +y, degrees"),
      twist: int("rotation about the pen's axis, degrees"),
      eraser: bool("the eraser end"),
      barrel: bool("the barrel button is held"),
      fromX: int("where the readings of a move start"),
      fromY: int("where the readings of a move start"),
      fromPressure: int("the pressure the readings ramp from (default: --pressure)"),
      steps: int("how many readings the move carries"),
      interval: int("milliseconds between the readings (default 4)"),
    },
    cli: (rest, flags) => {
      const base = compact({
        kind: flags.kind ? String(flags.kind) : "hover",
        pressure: num(flags.pressure),
        tiltX: num(flags["tilt-x"]),
        tiltY: num(flags["tilt-y"]),
        twist: num(flags.twist),
        eraser: flags.eraser ? true : undefined,
        barrel: flags.barrel ? true : undefined,
        fromX: num(flags["from-x"]),
        fromY: num(flags["from-y"]),
        fromPressure: num(flags["from-pressure"]),
        steps: num(flags.steps),
        interval: num(flags.interval),
      });
      if (rest[0]) return withIndex({ ...base, selector: rest[0] }, flags);
      const x = num(flags.x);
      const y = num(flags.y);
      if (x === undefined || y === undefined) die("pen needs a selector, or both --x and --y");
      return { ...base, x, y };
    },
  },
  {
    cmd: "scroll",
    description:
      "Scroll a container by (dx, dy) CSS pixels through its own gpui ScrollHandle and fire the " +
      "`scroll` event. The node must have `overflow: scroll|auto` in the cascade. This proves a " +
      "container CAN scroll and says nothing about the gesture — sk_wheel is the gesture.",
    usage: ["  scroll   <selector> <dy> [--dx N] [--index N]"],
    args: {
      selector: SELECTOR_REQ,
      dy: int("vertical delta, positive = down"),
      dx: int("horizontal delta"),
      index: INDEX,
    },
    cli: (rest, flags) => {
      if (!rest[0]) die("scroll needs a selector");
      return withIndex({ selector: rest[0], dx: num(flags.dx) ?? 0, dy: num(rest[1]) ?? num(flags.dy) ?? 0 }, flags);
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
      "  wheel    [selector] [--x N --y N] --dy N [--dx N] [--lines] [--ctrl] [--shift] [--alt]",
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
      lines: bool("the deltas count lines, as a notched mouse wheel reports them; a touchpad reports pixels"),
      ctrl: bool("Ctrl is held (a ctrl+wheel zooms in most apps)"),
      shift: bool("Shift is held"),
      alt: bool("Alt is held"),
    },
    cli: (rest, flags) => {
      const dy = num(flags.dy) ?? 0;
      const dx = num(flags.dx) ?? 0;
      if (!dy && !dx) die("wheel needs --dy (or --dx); DOM signs, so --dy 200 scrolls down");
      const keys = {
        ...(flags.ctrl ? { ctrl: true } : {}),
        ...(flags.shift ? { shift: true } : {}),
        ...(flags.alt ? { alt: true } : {}),
        ...(flags.lines ? { lines: true } : {}),
      };
      if (rest[0]) return withIndex({ selector: rest[0], dx, dy, ...keys }, flags);
      const x = num(flags.x);
      const y = num(flags.y);
      if (x === undefined || y === undefined) die("wheel needs a selector, or both --x and --y");
      return { dx, dy, x, y, ...keys };
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
      "Capture the app window (or the one `window` names) to a PNG. Default backend is gpui's render_to_image readback: the " +
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
        // A relative path on a CLI means relative to where it was typed; the host's cwd is its own.
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
      'so "why is this janky" can be asked of a host that is already running. `phases` reports ' +
      "whether SK_PERF's stderr breakdown is on rather than duplicating its counters (reading " +
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
      "SK_MEMSTATS=0/1 overrides), JSC's GC heap and mimalloc, GPU per adapter LUID, gpui's sprite " +
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
      compact({
        gc: flags.gc ? true : undefined,
        code: flags.code ? true : undefined,
        regions: flags.regions ? true : undefined,
      }),
  },
  {
    cmd: "trace",
    description:
      "Record instrumented main-thread elapsed time. recording-start/recording-stop/recording-status " +
      "control bounded ordered captures; frames pages summaries and frame pages nested calls with " +
      "parentId, selfMs, durationMs, counters, callsites and hints. These are CPU/host timings, " +
      "not GPU timestamps or sampled CPU cycles. Legacy start/stop/peek/mark retain their aggregate " +
      "phase report (and still refuse SK_PERF counter ownership conflicts).",
    usage: [
      "  trace    recording-start [--cap N] | recording-stop | recording-status",
      "  trace    frames [--offset N] [--limit N]",
      "  trace    frame --frame-id ID [--offset N] [--limit N]",
      "  trace    start [--cap N] | stop [--frames] | peek [--frames] | mark <label>",
    ],
    args: {
      op: str(
        "recording-start | recording-stop | recording-status | frames | frame; legacy start | stop | peek (default) | mark",
      ),
      frameId: int("for op=frame: a captured frame id"),
      offset: int("paged result offset, starting at zero"),
      limit: int("page size: frames <=240, frame spans <=1000"),
      frames: bool("legacy stop/peek: include every stored aggregate frame"),
      label: str("legacy mark: the label"),
      cap: int("recording-start: 16..10000 frames (default 1200); legacy start default 20000"),
    },
    cli: (rest, flags) => {
      const op = rest[0] || stringFlag(flags.op) || "peek";
      if (["recording-start", "recording-stop", "recording-status", "frames", "frame"].includes(op)) {
        const frameId = integerFlag(
          flags["frame-id"] ?? flags.frameId ?? (op === "frame" ? rest[1] : undefined),
          "--frame-id",
        );
        if (op === "frame" && frameId === undefined) die("trace frame needs --frame-id ID");
        return compact({
          op,
          frameId,
          offset: integerFlag(flags.offset, "--offset"),
          limit: integerFlag(flags.limit, "--limit", 1, op === "frames" ? 240 : 1000),
          cap: integerFlag(flags.cap, "--cap", 16, 10000),
        });
      }
      if (!["start", "stop", "peek", "mark"].includes(op)) die("unknown trace op");
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
      "Headers, bodies, cookies and timing are NOT here — pass a row's `seq` to sk_net_detail.",
    usage: ["  net      [--since N]               the fetch / WebSocket / XHR log"],
    args: { since: SINCE },
    cli: (_rest, flags) => compact({ since: num(flags.since) }),
  },
  {
    cmd: "net_detail",
    aliases: ["net-detail"],
    description:
      "Everything behind ONE sk_net row, asked for by its `seq`: the request (URL, method, the " +
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
      seq: { type: "number", description: "the row's `seq`, from sk_net", required: true },
    },
    cli: (rest, flags) => {
      const seq = num(rest[0]) ?? num(flags.seq);
      if (seq === undefined) die("net_detail needs a row's seq (from `net`): ctl net_detail 12");
      return { seq };
    },
  },
  {
    cmd: "net_cache",
    aliases: ["net-cache"],
    description:
      "The disk HTTP cache under fetch, XHR and every <img>/font load: its " +
      "folder, cap, entries and size, whether this instance owns the folder (`writable`) and seals " +
      "entries (`sealed`), and the counters since start (hits, revalidated, stored, misses, " +
      'bypassed, evicted, errors). `op: "clear"` deletes every entry and answers `freed` (bytes). ' +
      "`null` when no cache is installed (SK_HTTP_CACHE=0, httpCache.maxMB 0, or no storage).",
    usage: ["  net_cache [stats|clear]            the disk HTTP cache: counters, or empty it"],
    args: { op: str('"stats" (default) or "clear"') },
    cli: (rest, flags) => compact({ op: rest[0] ?? (flags.op as string | undefined) }),
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
    cmd: "indexeddb",
    description:
      "Read-only IndexedDB inspection without opening/upgrading databases or running application JS. " +
      "list returns names/versions; schema returns stores, indexes, key paths and counts; records " +
      "returns bounded typed structured-clone previews, primary/index keys and an opaque nextCursor. " +
      "Blob bodies are not searched. Busy databases report an error to retry, never an upgrade.",
    usage: [
      "  indexeddb [list]                    database names and versions",
      "  indexeddb schema <database>         object stores and indexes",
      "  indexeddb records <database> <store> [--index NAME] [--after CURSOR]",
      "            [--limit N] [--search TEXT] bounded typed previews (maximum 100 records)",
    ],
    args: {
      op: str('"list" (default), "schema" or "records"'),
      database: str("database name"),
      store: str("object store name"),
      index: str("optional index name; absent walks primary keys"),
      after: str("opaque hexadecimal nextCursor from the preceding page"),
      limit: int("records per page, 1..100 (default 50)"),
      search: str("key and bounded preview search, up to 256 characters; excludes Blob bodies"),
    },
    cli: (rest, flags) => {
      const op = rest[0] || stringFlag(flags.op) || "list";
      if (!["list", "schema", "records"].includes(op)) die("indexeddb op must be list, schema or records");
      const database = stringFlag(flags.database) ?? rest[1],
        store = stringFlag(flags.store) ?? rest[2];
      if (op !== "list" && database === undefined) die(`indexeddb ${op} needs a database name`);
      if (op === "records" && store === undefined) die("indexeddb records needs an object store name");
      const search = stringFlag(flags.search),
        after = stringFlag(flags.after);
      if (search !== undefined && [...search].length > 256) die("--search is limited to 256 characters");
      if (after !== undefined && (!/^[a-f0-9]*$/i.test(after) || after.length % 2 !== 0))
        die("--after must be the opaque hexadecimal nextCursor");
      return compact({
        op,
        database,
        store,
        index: stringFlag(flags.index),
        after,
        search,
        limit: integerFlag(flags.limit, "--limit", 1, 100),
      });
    },
  },
  {
    cmd: "allocations",
    description:
      "Bounded Rust allocator event capture: start/stop/status/snapshot, explicit scoped sites and " +
      "size classes, observed churn and capture-born survivors. Pointer-hash sampling is not " +
      "extrapolated; coverage states missing JS, JSC and native allocator hooks. Optional heap " +
      "counter is separate from captured events. Dropped events/partial rows make complete=false.",
    usage: [
      "  allocations [status|snapshot] [--category TEXT] [--site TEXT]",
      "              [--min-size N] [--max-size N] [--limit N]",
      "  allocations start [--sample-every N] [--max-events N] | stop",
    ],
    args: {
      op: str('"status" (default), "start", "stop" or "snapshot"'),
      sampleEvery: int("pointer-hash sample interval, 1..4096; observed counts are not extrapolated"),
      maxEvents: int("event buffer capacity, 1024..65536"),
      category: str("category substring filter"),
      site: str("explicit site substring filter"),
      minSize: int("minimum allocation size in bytes"),
      maxSize: int("maximum allocation size in bytes"),
      limit: int("maximum grouped rows, 1..2000 (default 300)"),
    },
    cli: (rest, flags) => {
      const op = rest[0] || stringFlag(flags.op) || "status";
      if (!["start", "stop", "status", "snapshot"].includes(op))
        die("allocations op must be start, stop, status or snapshot");
      const minSize = integerFlag(flags["min-size"] ?? flags.minSize, "--min-size"),
        maxSize = integerFlag(flags["max-size"] ?? flags.maxSize, "--max-size");
      if (minSize !== undefined && maxSize !== undefined && minSize > maxSize) die("--min-size exceeds --max-size");
      return compact({
        op,
        minSize,
        maxSize,
        category: stringFlag(flags.category),
        site: stringFlag(flags.site),
        sampleEvery: integerFlag(flags["sample-every"] ?? flags.sampleEvery, "--sample-every", 1, 4096),
        maxEvents: integerFlag(flags["max-events"] ?? flags.maxEvents, "--max-events", 1024, 65536),
        limit: integerFlag(flags.limit, "--limit", 1, 2000),
      });
    },
  },
  {
    cmd: "dialog",
    description:
      "The dialogs waiting for an answer — <input type=file>, the File System Access pickers, " +
      "<a download>'s Save As, and the app's permission prompt — when the app runs with " +
      "SK_DIALOGS=scripted (nothing reaches the OS then). `list` names each one ({id, window, " +
      "what, kind, ...}: a file dialog's multiple, filters, startIn, suggestedName; a question's " +
      '(kind "ask") instruction and buttons); `answer` finishes a file dialog with absolute ' +
      "paths, checked as the OS dialog would check them; `choose` presses a question's button " +
      '(its label, e.g. "Allow", or its index); `cancel` dismisses either, as Esc would. ' +
      "Without SK_DIALOGS=scripted every dialog is real and this command refuses.",
    usage: [
      "  dialog   [list]                     dialogs waiting for an answer (SK_DIALOGS=scripted)",
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
      'appPermissions.request("microphone") are the ways back after Block. It changes a real ' +
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
    cmd: "effects",
    description:
      "`--shading` effects: the effects.json schemas the host loaded (built-ins and the app's own), and " +
      "every element painting one with its declared text, resolved parameters and the rules that declare " +
      "it. `set` overrides an element's `--shading` inline, live, and answers the advisories the value " +
      "raises; `reset` puts back what the app had.",
    usage: [
      "  effects [list]                      schemas and the elements using them",
      "  effects node --node N               one element's effect",
      "  effects set --node N --value TEXT   override live, e.g. --value 'frost(0.6, 14px)'",
      "  effects reset --node N              drop the override",
    ],
    args: {
      op: str('"list" (default), "node", "set" or "reset"'),
      node: NODE,
      value: str("for op=set: the `--shading` value, e.g. frost(0.6, 14px) or none"),
    },
    cli: (rest, flags) =>
      compact({
        op: rest[0] || "list",
        node: num(flags.node ?? rest[1]),
        value: stringFlag(flags.value),
      }),
  },
  {
    cmd: "raise",
    description:
      "Bring the addressed window to the front. The ONLY command here that deliberately takes " +
      "focus — SK_DEVTOOLS_RAISE=off in the target's environment suppresses the activation, for " +
      "an automated run on a machine somebody is using.",
    usage: ["  raise                              bring the window to the front"],
    args: {},
    cli: () => ({}),
  },
];

export interface McpTool {
  name: string;
  description: string;
  inputSchema: {
    type: "object";
    properties: Record<string, { type: string; description: string }>;
    required?: string[];
  };
  build: (args: Record<string, unknown>) => Record<string, unknown>;
}

export const toolName = (cmd: string): string => `sk_${cmd}`;

export function mcpTools(): McpTool[] {
  return COMMANDS.map((spec) => {
    const names = Object.keys(spec.args);
    const required = names.filter((n) => spec.args[n]!.required);
    const properties: Record<string, { type: string; description: string }> = {};
    for (const n of names) {
      const arg = spec.args[n]!;
      properties[n] = { type: arg.type, description: arg.description };
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

export const commandNames = (): string[] => COMMANDS.map((c) => c.cmd);

export function usageBlock(): string {
  return COMMANDS.flatMap((c) => c.usage).join("\n");
}

/** `["query", ".tab"]` + flags -> `{cmd: "query", selector: ".tab"}`. Throws [`UsageError`] for an unknown verb or malformed invocation. */
/** Split actual CLI argv using the same boolean arguments the MCP schema exposes. */
export function splitArgs(argv: string[]): { positional: string[]; flags: Flags } {
  const switches = new Set([
    "json",
    "help",
    ...COMMANDS.flatMap((spec) =>
      Object.entries(spec.args)
        .filter(([, arg]) => arg.type === "boolean")
        .map(([name]) => name),
    ),
  ]);
  const positional: string[] = [];
  const flags: Flags = {};
  for (let i = 0; i < argv.length; i++) {
    const value = argv[i]!;
    if (!value.startsWith("--")) positional.push(value);
    else {
      const name = value.slice(2);
      flags[name] = switches.has(name) ? true : (argv[++i] ?? "");
    }
  }
  return { positional, flags };
}

export function buildRequest(argv: string[], flags: Flags): Record<string, unknown> {
  const [cmd, ...rest] = argv;
  const spec = COMMANDS.find((c) => c.cmd === cmd || (cmd !== undefined && (c.aliases ?? []).includes(cmd)));
  if (!spec) die(`unknown command \`${cmd}\``);
  return { cmd: spec.cmd, ...spec.cli(rest, flags) };
}
