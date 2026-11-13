// The Locator — a selector plus the patience to wait for it.

import type { Bounds, NodeInfo, VnApp } from "./app";
import { VnCommandError } from "./wire";

export interface LocatorOptions {
  timeout?: number;
  /** Which of several matches this locator means. */
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
    return i === undefined
      ? `locator(${JSON.stringify(this.selector)})`
      : `locator(${JSON.stringify(this.selector)}).nth(${i})`;
  }

  nth(index: number): Locator {
    return new Locator(this.app, this.selector, { ...this.options, index });
  }
  first(): Locator {
    return this.nth(0);
  }

  /** How many nodes match RIGHT NOW. */
  async count(): Promise<number> {
    const { count } = await this.app.query(this.selector);
    return count;
  }

  /** Every matching node as the host reports it, right now. */
  async all(): Promise<NodeInfo[]> {
    const { nodes } = await this.app.query(this.selector);
    return nodes;
  }

  // -- the wait

  /** Polls until the selector resolves to exactly one painted node, or the timeout runs out. */
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

  /** ONE attempt: no waiting, no artifacts, no throw for anything a later frame could fix. */
  async peek(): Promise<Resolved | { why: string }> {
    let reply: { count: number; nodes: NodeInfo[] };
    try {
      reply = await this.app.query(this.selector);
    } catch (e) {
      if (e instanceof VnCommandError) {
        // The matcher rejected the SELECTOR — retrying cannot help, so this is the one failure
        // that escapes a non-throwing probe.
        throw new LocatorError(`${this.description}: ${e.hostError}`);
      }
      throw e;
    }
    const { count, nodes } = reply;
    const index = this.options.index;
    if (count === 0) return { why: "matched no nodes" };
    if (index === undefined && count > 1) {
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

  /** True when the node exists and was painted last frame. */
  async isVisible(): Promise<boolean> {
    try {
      const { nodes } = await this.app.query(this.selector);
      return painted(nodes[this.options.index ?? 0]);
    } catch {
      return false;
    }
  }

  /** Clicks it. */
  async click(options: { real?: boolean; button?: number; timeout?: number } = {}): Promise<void> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    if (options.real) {
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

  /** Focuses the node and delivers one real keystroke per character. */
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
  async scroll(
    dy: number,
    dx = 0,
    options: { timeout?: number } = {},
  ): Promise<{ scrollLeft: number; scrollTop: number }> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    return this.app.send("scroll", { selector: this.selector, index, dx, dy });
  }

  /**
   * The subtree's text — or a native `<input>`'s live buffer, which is where the text of a field
   * actually lives (its `textContent` would be `""`, and that would be a lie).
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

  /** The node's computed styles, bucket by bucket. */
  async styles(options: { timeout?: number; settlePointer?: boolean } = {}): Promise<{
    id: number;
    hovered: boolean;
    computed: Record<string, string[]>;
  }> {
    const { index } = await this.resolve(options.timeout ?? this.timeout);
    // Both `hovered` and the `live` bucket are answers about gpui's hit test, and gpui hit-tests
    // against a `mouse_position` the person at the machine can overwrite at any moment.
    if (options.settlePointer !== false) await this.app.settlePointer();
    return this.app.send("styles", { selector: this.selector, index });
  }

  /** A screenshot of the whole window, taken once this locator resolves. */
  async screenshot(path?: string, options: { timeout?: number } = {}): Promise<string> {
    await this.resolve(options.timeout ?? this.timeout);
    return this.app.screenshot(path);
  }
}
