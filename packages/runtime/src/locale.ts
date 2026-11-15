export type Messages = Readonly<Record<string, Readonly<Record<string, string>>>>;
export type MessageArgs = Readonly<Record<string, string | number>>;

export interface AppLocale {
  /** The language the SDK's dialogs and `t` use: `language` if set, else the best match for `navigator.languages`. */
  readonly resolved: string;
  /** `null` follows `navigator.languages`. */
  language: string | null;
  readonly available: readonly string[];
  /** `{ "de": { "app.hello": "Hallo {name}" } }`; `sk.*` keys reword or translate the SDK's own text. */
  define(messages: Messages): void;
  t(key: string, args?: MessageArgs): string;
}

declare const __sk_appLocale: AppLocale;
export const appLocale: AppLocale = __sk_appLocale;
