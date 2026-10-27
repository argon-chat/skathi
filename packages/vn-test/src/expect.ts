// Web-first assertions: matchers that RETRY.
//
// `expect(await el.textContent()).toBe("2")` samples once and races the app.
// `await expect(el).toHaveText("2")` keeps sampling until it agrees or the
// timeout runs out — which is the difference between a suite that is green and
// a suite that is green today. Playwright calls these web-first assertions;
// there is nothing web about the idea, only about where it was named.
//
// This module exports its own `expect`. It could have gone through bun's
// `expect.extend`, but that API is built around a synchronous `pass`/`message`
// pair — a retrying matcher is asynchronous by nature, and the ones that
// squeeze through end up reporting a stale `actual` from before the last poll.
// A plain function that DELEGATES anything that is not ours to `bun:test`'s
// `expect` costs one import and keeps both halves honest:
//
//     import { expect } from "vn-test";
//     expect(2 + 2).toBe(4);                  // bun:test, unchanged
//     await expect(page.locator(".x")).toHaveText("hi");   // ours, retrying

import { expect as bunExpect } from "bun:test";
import { Locator, POLL_MS } from "./locator";
import { styleKeys, styleValue } from "./styles";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export interface AssertOptions {
  timeout?: number;
}

type Probe = () => Promise<{ pass: boolean; actual: string }>;

/** Matches a string against a string (equality) or a RegExp (test). */
const matches = (actual: string, expected: string | RegExp) =>
  typeof expected === "string" ? actual === expected : expected.test(actual);

const show = (v: unknown) => (v instanceof RegExp ? String(v) : JSON.stringify(v));

export class LocatorAssertions {
  constructor(
    private readonly locator: Locator,
    private readonly isNot: boolean,
  ) {}

  /** Inverts the next matcher. Polls until the condition stops holding. */
  get not(): LocatorAssertions {
    return new LocatorAssertions(this.locator, !this.isNot);
  }

  /**
   * The retry loop every matcher below shares.
   *
   * A probe that THROWS counts as "not yet" rather than as a failure: a locator
   * that has not appeared, or a `styles` call on a node the cascade has not
   * reached, is exactly the transient the retry exists for. The last message is
   * kept and reported when the deadline passes, so the failure still says what
   * went wrong rather than just "timed out".
   */
  async #poll(what: string, expected: string, options: AssertOptions, probe: Probe): Promise<void> {
    const timeout = options.timeout ?? this.locator.timeout;
    const deadline = Date.now() + timeout;
    let actual = "(never sampled)";
    for (;;) {
      let pass = false;
      try {
        const r = await probe();
        pass = r.pass;
        actual = r.actual;
      } catch (e) {
        pass = false;
        actual = `threw: ${(e as Error).message.split("\n")[0]}`;
      }
      if (pass !== this.isNot) return;
      if (Date.now() >= deadline) throw await this.#fail(what, expected, actual, timeout);
      await sleep(POLL_MS);
    }
  }

  async #fail(what: string, expected: string, actual: string, timeout: number): Promise<Error> {
    const app = this.locator.app;
    const shot = await app.screenshot().catch((e) => `(screenshot failed: ${(e as Error).message})`);
    const logs = await app.tailLogs(10);
    const e = new Error(
      `${this.locator.description}\n` +
        `  expected ${this.isNot ? "NOT " : ""}${what}: ${expected}\n` +
        `  actual:   ${actual}\n` +
        `  waited:   ${timeout} ms\n` +
        `  screenshot: ${shot}\n` +
        `  host log:   ${app.hostLog}\n` +
        `  last log lines:\n${logs}`,
    );
    e.name = "VnAssertionError";
    // The frames inside this file are noise — the useful line is the spec's.
    Error.captureStackTrace?.(e, LocatorAssertions.prototype.constructor);
    return e;
  }

  // -- presence -------------------------------------------------------------

  /** Exists and was painted last frame with a non-empty box. */
  async toBeVisible(options: AssertOptions = {}): Promise<void> {
    await this.#poll("visible", "true", options, async () => {
      const r = await this.locator.peek();
      return "node" in r
        ? { pass: true, actual: `visible at ${JSON.stringify(r.node.bounds)}` }
        : { pass: false, actual: r.why.split("\n")[0] };
    });
  }

  /** The negative of `toBeVisible`, spelled the way a reader expects. */
  async toBeHidden(options: AssertOptions = {}): Promise<void> {
    await this.not.toBeVisible(options);
  }

  /**
   * How many nodes the selector matches. Counts ALL of them — `.first()` and
   * `.nth()` do not narrow it, because "how many are there" is a question about
   * the selector, not about the one you picked.
   */
  async toHaveCount(expected: number, options: AssertOptions = {}): Promise<void> {
    await this.#poll("count", String(expected), options, async () => {
      const count = await this.locator.count();
      return { pass: count === expected, actual: String(count) };
    });
  }

  // -- text -----------------------------------------------------------------

  /**
   * The whole text, trimmed. Inner whitespace is collapsed the way Playwright's
   * `toHaveText` does, so a template that wraps a line does not change the
   * answer.
   */
  async toHaveText(expected: string | RegExp, options: AssertOptions = {}): Promise<void> {
    await this.#poll("text", show(expected), options, async () => {
      const text = normalize(await this.locator.textContent(options));
      return { pass: matches(text, expected), actual: JSON.stringify(text) };
    });
  }

  async toContainText(expected: string | RegExp, options: AssertOptions = {}): Promise<void> {
    await this.#poll("text containing", show(expected), options, async () => {
      const text = normalize(await this.locator.textContent(options));
      const pass = typeof expected === "string" ? text.includes(expected) : expected.test(text);
      return { pass, actual: JSON.stringify(text) };
    });
  }

  // -- classes --------------------------------------------------------------

  /**
   * `string` — the class list must contain it (this is the useful test 95 % of
   * the time and what a spec means by `toHaveClass("router-link-active")`).
   * `string[]` — the list must contain every one of them.
   * `RegExp` — tested against the joined list.
   */
  async toHaveClass(expected: string | string[] | RegExp, options: AssertOptions = {}): Promise<void> {
    await this.#poll("class", show(expected), options, async () => {
      const classes = await this.locator.classes(options);
      const joined = classes.join(" ");
      const pass =
        expected instanceof RegExp
          ? expected.test(joined)
          : (Array.isArray(expected) ? expected : [expected]).every((c) => classes.includes(c));
      return { pass, actual: JSON.stringify(joined) };
    });
  }

  // -- style ----------------------------------------------------------------

  /**
   * A property of the element's computed style.
   *
   * The default bucket is `live` — the style IN EFFECT this frame, which is what
   * `getComputedStyle()` means and the only bucket that can prove a `:hover`
   * landed. Pass `{ bucket: "base" }` for the resting cascade, or
   * `{ bucket: "hover" }` to assert on the RULE rather than on its effect.
   *
   * `property` is the engine's field name (`background`, `border_color`,
   * `opacity`), with a few CSS spellings aliased — see `src/styles.ts`.
   * `expected` is a substring of the flattened value, or a RegExp against it;
   * a colour is easiest asserted by its distinguishing component,
   * e.g. `toHaveComputedStyle("background", "a: 0.8")`.
   */
  async toHaveComputedStyle(
    property: string,
    expected: string | RegExp,
    options: AssertOptions & { bucket?: string } = {},
  ): Promise<void> {
    const bucket = options.bucket ?? "live";
    await this.#poll(`computed[${bucket}].${property}`, show(expected), options, async () => {
      const { computed } = await this.locator.styles(options);
      const lines = computed[bucket];
      if (!lines) {
        return {
          pass: false,
          actual: `no ${JSON.stringify(bucket)} bucket (have: ${Object.keys(computed).join(", ")})`,
        };
      }
      const value = styleValue(lines, property);
      if (value === undefined) {
        return {
          pass: false,
          actual: `${property} is not declared (bucket declares: ${styleKeys(lines).join(", ")})`,
        };
      }
      const pass = typeof expected === "string" ? value.includes(expected) : expected.test(value);
      return { pass, actual: JSON.stringify(value) };
    });
  }

  /**
   * The pointer is inside this element's hitbox, as gpui's own hover edges
   * report it.
   *
   * Only ever true for an element the renderer asked for hover edges on — one
   * with a `:hover` rung AND a `transition`, or a `mouseenter`/`mouseleave`
   * listener. A `:hover` rule with no transition is applied inside gpui's paint
   * ladder and never reaches the tree, so this stays false for it. See
   * docs/TESTING.md.
   */
  async toBeHovered(options: AssertOptions = {}): Promise<void> {
    await this.#poll("hovered", "true", options, async () => {
      // This assertion's SUBJECT is the pointer, so it is placed before it is
      // read even when the spec has not hovered yet — `not.toBeHovered()`
      // before the first `hover()` is a claim about where the pointer is, and
      // reading whatever the live user left there is how that claim used to
      // become a coin flip. `styles()` re-places it on every poll.
      await this.locator.app.settlePointer(true);
      const { hovered } = await this.locator.styles(options);
      return { pass: !!hovered, actual: String(!!hovered) };
    });
  }
}

const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

// ---------------------------------------------------------------------------

/**
 * `expect` for both worlds: ours when handed a `Locator`, `bun:test`'s for
 * everything else, so a spec needs exactly one import.
 *
 * There is no app-level `expect(app)`: the only assertion a whole app has is
 * "it logged nothing broken", and that reads better as `app.expectNoErrors()`
 * than as a matcher on a thing that is not a value.
 */
export const expect: any = (actual: any) => {
  if (actual instanceof Locator) return new LocatorAssertions(actual, false);
  return bunExpect(actual);
};

// Anything bun hangs off `expect` (`expect.any`, `expect.objectContaining`,
// `expect.extend`, …) has to survive the wrapper, or importing ours would
// quietly delete half of bun's API.
for (const key of Reflect.ownKeys(bunExpect)) {
  if (key === "name" || key === "length" || key === "prototype") continue;
  try {
    Object.defineProperty(expect, key, Object.getOwnPropertyDescriptor(bunExpect, key)!);
  } catch {}
}

/** The same function under the name to reach for when `expect` is already taken. */
export const vnExpect = expect;
