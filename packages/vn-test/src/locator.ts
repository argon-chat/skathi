// The Locator — a selector plus the patience to wait for it.
//
// Auto-waiting is the feature this package exists for. Everything else the
// devtools protocol already gave us; what it did not give us is "keep asking
// until the app catches up", and without that every spec degenerates into
// hand-placed sleeps that are either flaky or slow.
//
// The poll is CLIENT-side and 50 ms, over the wire that already exists. That is
// a deliberate v1 choice, not an oversight: a `query` round trip on localhost
// costs well under a millisecond of wall clock and one frame of the host's
// budget, so a 10 s timeout is at most ~200 frames of extra work spread over
// ten seconds — invisible next to the app's own 60 Hz pump. A host-side `wait`
// command would save those round trips and buy nothing a test can feel.
//
// ACTIONABILITY, honestly: before acting we require the node to EXIST and to
// have been PAINTED (non-null bounds with a non-zero box). Playwright checks
// four more things — stable (two animation frames at the same position),
// receives-events (hit test at the action point), enabled, and editable — and
// three of those have no answer on this stack at all. docs/TESTING.md has the
// table; `click({ real: true })` is the escape hatch that buys the hit test.

import type { Bounds, NodeInfo, VnApp } from "./app";
import { VnCommandError } from "./wire";

export interface LocatorOptions {
  timeout?: number;
  /** Which of several matches this locator means. Set by `first()` / `nth()`. */
  index?: number;
}

export const POLL_MS = 50;

/** Resolved once a locator has settled on a node: the node and where it sat. */
export type Resolved = { node: NodeInfo; index: number; count: number };

export class LocatorError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "LocatorError";
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const painted = (n: NodeInfo | undefined): n is NodeInfo & { bounds: Bounds } =>
  !!n && !n.gone && !!n.bounds && n.bounds.w > 0 && n.bounds.h > 0;

export class Locator {
  constructor(
    readonly app: VnApp,
    readonly selector: string,
    readonly options: LocatorOptions = {},
  ) {}

  get timeout() {
    return this.options.timeout ?? this.app.timeout;
  }

  /** A description of this locator for a failure message. */
  get description() {
    const i = this.options.index;
    return i === undefined ? `locator(${JSON.stringify(this.selector)})` : `locator(${JSON.stringify(this.selector)}).nth(${i})`;
  }

  // -- narrowing ------------------------------------------------------------

  nth(index: number): Locator {
    return new Locator(this.app, this.selector, { ...this.options, index });
  }
  first(): Locator {
    return this.nth(0);
  }

  // -- reading (no waiting) -------------------------------------------------

  /** How many nodes match RIGHT NOW. Does not wait — that is `expect(...).toHaveCount`. */
  async count(): Promise<number> {
    const { count } = await this.app.query(this.selector);
    return count;
  }

  /** Every matching node as the host reports it, right now. */
  async all(): Promise<NodeInfo[]> {
    const { nodes } = await this.app.query(this.selector);
    return nodes;
  }

  // -- the wait ------------------------------------------------------------

  /**
   * Polls until the selector resolves to exactly one painted node, or the
   * timeout runs out.
   *
   * STRICTNESS is Playwright's rule and also the host's: a selector that
   * matches several things is ambiguous, and silently taking the first one is
   * how a test passes for the wrong reason. `first()` / `nth()` is how you say
   * you meant it. (The `click` command errors the same way for the same reason
   * — see docs/DEVTOOLS.md.)
   *
   * A selector the CASCADE cannot express is not a timing problem, so it throws
   * immediately, carrying the engine's own advisory.
   */
  async resolve(timeoutMs = this.timeout): Promise<Resolved> {
    const deadline = Date.now() + timeoutMs;
    let last = "never queried";
    for (;;) {
      const attempt = await this.peek();
      if ("node" in attempt) return attempt;
      last = attempt.why;
      if (Date.now() >= deadline) {
        throw new LocatorError(
          `${this.description} did not resolve within ${timeoutMs} ms — ${last}\n` +
            (await this.app.tailLogs(8)) +
            `\n  screenshot: ${await this.app.screenshot().catch(() => "(unavailable)")}`,
        );
      }
      await sleep(POLL_MS);
    }
  }

  /**
   * ONE attempt: no waiting, no artifacts, no throw for anything a later frame
   * could fix. This is what the retrying matchers poll on — `resolve()` would
   * take a screenshot on every tick, which turns a 10 s assertion into 200
   * PNGs.
   */
  async peek(): Promise<Resolved | { why: string }> {
    let reply: { count: number; nodes: NodeInfo[] };
    try {
      reply = await this.app.query(this.selector);
    } catch (e) {
      if (e instanceof VnCommandError) {
        // The matcher rejected the SELECTOR — retrying cannot help, so this is
        // the one failure that escapes a non-throwing probe.
        throw new LocatorError(`${this.description}: ${e.hostError}`);
      }
      throw e;
    }
    const { count, nodes } = reply;
    const index = this.options.index;
    if (count === 0) return { why: "matched no nodes" };
    if (index === undefined && count > 1) {
      // Strict mode. Report the candidates: "which one did you mean" is
      // answerable only if the message says what there was to choose from.
      return {
        why:
          `matched ${count} nodes — add .first() or .nth(i):\n` +
          nodes
            .slice(0, 8)
            .map(
              (n, i) =>
                `      [${i}] <${n.tag}${(n.classes ?? []).map((c) => `.${c}`).join("")}> ` +
                JSON.stringify((n.text ?? "").slice(0, 40)),
            )
            .join("\n"),
      };
    }
    const i = index ?? 0;
    const node = nodes[i];
    if (!node) return { why: `matched ${count} node(s) — index ${i} is out of range` };
    if (!painted(node)) {
      return { why: `node ${node.id} is not painted (bounds ${JSON.stringify(node.bounds)})` };
    }
    return { node, index: i, count };
  }

  /** Waits for the node to be there and painted; returns it. */
  async waitFor(options: { timeout?: number } = {}): Promise<NodeInfo> {
    return (await this.resolve(options.timeout ?? this.timeout)).node;
  }

  /** True when the node exists and was painted last frame. Never throws. */
  async isVisible(): Promise<boolean> {
    try {
      const { nodes } = await this.app.query(this.selector);
      return painted(nodes[this.options.index ?? 0]);
    } catch {
      return false;
    }
  }

  // -- actions --------------------------------------------------------------

  /**
   * Clicks it.
   *
   * By DEFAULT this is the host's `click`: mousedown → mouseup → click
   * synthesized into the app's own dispatcher from the resolved node id, so it
   * exercises the capture/bubble chain and honours `preventDefault()`. It
   * proves the HANDLER, and it can never prove the element was reachable.
   *
   * `{ real: true }` sends a real `PlatformInput` at the node's centre instead,
   * through gpui's hit test — so z-order, content masks and `pointer-events`
   * all get a vote, and a control hidden under something else FAILS, which is
   * exactly what you want when reachability is the thing under test. It is not
   * the default because most specs are driving an app, not auditing its z-order.
   */
  async click(options: { real?: boolean; button?: number; timeout?: number } = {}): Promise<void> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    if (options.real) {
      // A real click moves the pointer onto the node first, so it IS a hover
      // gesture as far as gpui is concerned — record it, or the next style
      // read would re-place the pointer somewhere else and undo it.
      this.app.notePointer({ kind: "node", selector: this.selector, index });
      await this.app.send("mouse", {
        selector: this.selector,
        index,
        kind: "click",
        ...(options.button !== undefined ? { button: options.button } : {}),
      });
      return;
    }
    await this.app.send("click", {
      selector: this.selector,
      index,
      ...(options.button !== undefined ? { button: options.button } : {}),
    });
  }

  /**
   * Moves the real pointer onto the node's centre — the only gesture that can
   * make `:hover` happen, because the hover ladder is decided by gpui's hit
   * test and not by anything we could synthesize. No cursor moves on the
   * screen and no window is activated.
   *
   * The placement is also REMEMBERED, and that half is what makes a hover
   * assertion mean anything: gpui hit-tests against one `mouse_position` per
   * window, fed by every `PlatformInput` move — ours and the live user's
   * alike — so every later read that depends on the hit test re-places the
   * pointer first (`VnApp.settlePointer`). Without it a hand on the mouse
   * decides the result, which is exactly the flake this file used to have.
   */
  async hover(options: { timeout?: number } = {}): Promise<{ x: number; y: number; focused: number | null }> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    this.app.notePointer({ kind: "node", selector: this.selector, index });
    return this.app.send("mouse", { selector: this.selector, index, kind: "move" });
  }

  /** Moves the pointer away from everything, so a previous `hover()` ends. */
  async unhover(): Promise<void> {
    this.app.notePointer({ kind: "point", x: 1, y: 1 });
    await this.app.send("mouse", { x: 1, y: 1, kind: "move" });
  }

  /**
   * Focuses the node and delivers one real keystroke per character.
   *
   * These are not synthesized: they go through `Window::dispatch_keystroke`, so
   * the widget's interceptor fires `keydown` into JS, the app's own
   * `preventDefault()` still cancels the character, and `input` fires from the
   * platform input handler. It APPENDS at the caret — there is no `fill()`,
   * because there is no host command that replaces a field's contents.
   */
  async type(text: string, options: { timeout?: number } = {}): Promise<void> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    await this.app.send("type", { selector: this.selector, index, text }, Math.max(this.timeout, 20_000));
  }

  /** One keystroke into this node: `"Enter"`, `"Shift+Tab"`, `"ArrowDown"`. */
  async press(key: string, options: { timeout?: number } = {}): Promise<void> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    await this.app.send("key", { selector: this.selector, index, key }, Math.max(this.timeout, 20_000));
  }

  async focus(options: { timeout?: number } = {}): Promise<void> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    await this.app.send("focus", { selector: this.selector, index });
  }

  /** Drives the element's own `ScrollHandle` and fires `scroll`. */
  async scroll(dy: number, dx = 0, options: { timeout?: number } = {}): Promise<{ scrollLeft: number; scrollTop: number }> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    return this.app.send("scroll", { selector: this.selector, index, dx, dy });
  }

  // -- values ---------------------------------------------------------------

  /**
   * The subtree's text — or a native `<input>`'s live buffer, which is where
   * the text of a field actually lives (its `textContent` would be `""`, and
   * that would be a lie).
   */
  async textContent(options: { timeout?: number } = {}): Promise<string> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    const data = await this.app.send("text", { selector: this.selector, index });
    return data.text as string;
  }

  async classes(options: { timeout?: number } = {}): Promise<string[]> {
    const { node } = await this.resolve(options.timeout ?? this.timeout);
    return node.classes ?? [];
  }

  async boundingBox(options: { timeout?: number } = {}): Promise<Bounds> {
    const { node } = await this.resolve(options.timeout ?? this.timeout);
    return node.bounds as Bounds;
  }

  /**
   * The node's computed styles, bucket by bucket.
   *
   * `computed.live` is the style IN EFFECT this frame — base, plus the
   * animator's in-flight overlay, plus the hover rung while the pointer is
   * actually over it. `computed.base` / `computed.hover` are what the CASCADE
   * produced, which is true whether or not anything is hovering. `hovered`
   * says which world you are in.
   */
  async styles(options: { timeout?: number; settlePointer?: boolean } = {}): Promise<{
    id: number;
    hovered: boolean;
    computed: Record<string, string[]>;
  }> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    // Both `hovered` and the `live` bucket are answers about gpui's hit test,
    // and gpui hit-tests against a `mouse_position` the person at the machine
    // can overwrite at any moment. So the last word on where the pointer is
    // belongs to the harness: it goes back where this spec put it,
    // immediately before the read. `settlePointer: false` opts out for a
    // caller that genuinely wants whatever is there.
    if (options.settlePointer !== false) await this.app.settlePointer();
    return this.app.send("styles", { selector: this.selector, index });
  }

  /** A screenshot of the whole window, taken once this locator resolves. */
  async screenshot(path?: string, options: { timeout?: number } = {}): Promise<string> {
    await this.resolve(options.timeout ?? this.timeout);
    return this.app.screenshot(path);
  }
}
