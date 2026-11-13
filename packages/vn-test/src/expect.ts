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

  /** Inverts the next matcher. */
  get not(): LocatorAssertions {
    return new LocatorAssertions(this.locator, !this.isNot);
  }

  /** The retry loop every matcher below shares. */
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
    Error.captureStackTrace?.(e, LocatorAssertions.prototype.constructor);
    return e;
  }

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

  /** How many nodes the selector matches. */
  async toHaveCount(expected: number, options: AssertOptions = {}): Promise<void> {
    await this.#poll("count", String(expected), options, async () => {
      const count = await this.locator.count();
      return { pass: count === expected, actual: String(count) };
    });
  }

  /** The whole text, trimmed. */
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

  /**
   * `string` — the class list must contain it (this is the useful test 95 % of the time and what a
   * spec means by `toHaveClass("router-link-active")`).
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

  /** A property of the element's computed style. */
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

  /** The pointer is inside this element's hitbox, as gpui's own hover edges report it. */
  async toBeHovered(options: AssertOptions = {}): Promise<void> {
    await this.#poll("hovered", "true", options, async () => {
      await this.locator.app.settlePointer(true);
      const { hovered } = await this.locator.styles(options);
      return { pass: !!hovered, actual: String(!!hovered) };
    });
  }
}

const normalize = (s: string) => s.replace(/\s+/g, " ").trim();

/**
 * `expect` for both worlds: ours when handed a `Locator`, `bun:test`'s for everything else, so a
 * spec needs exactly one import.
 */
export const expect: any = (actual: any) => {
  if (actual instanceof Locator) return new LocatorAssertions(actual, false);
  return bunExpect(actual);
};

// Anything bun hangs off `expect` (`expect.any`, `expect.objectContaining`, `expect.extend`, …)
// has to survive the wrapper, or importing ours would quietly delete half of bun's API.
for (const key of Reflect.ownKeys(bunExpect)) {
  if (key === "name" || key === "length" || key === "prototype") continue;
  try {
    Object.defineProperty(expect, key, Object.getOwnPropertyDescriptor(bunExpect, key)!);
  } catch {}
}

/** The same function under the name to reach for when `expect` is already taken. */
export const vnExpect = expect;
