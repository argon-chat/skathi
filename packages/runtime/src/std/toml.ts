// `skathi:toml`: TOML 1.1 in the shape of `JSON`. The host installs the object.

export interface Toml {
  /**
   * A TOML document as an object. Integers beyond ±2^53 are `bigint`; a date-time with an offset
   * is a `Date`, the local forms are `Temporal.PlainDateTime`, `PlainDate` and `PlainTime`.
   * Throws a `SyntaxError` with the line and column.
   */
  parse(text: string): Record<string, unknown>;
  /**
   * An object as a TOML document. `undefined`, functions and symbols are left out, as by
   * `JSON.stringify`; `null` throws a `TypeError` naming the key, since TOML has no null.
   */
  stringify(value: Record<string, unknown>, options?: { pretty?: boolean }): string;
}

declare const __sk_std: { TOML: Toml };

export const TOML: Toml = __sk_std.TOML;
