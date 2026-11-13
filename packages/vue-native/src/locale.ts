export type Messages = Readonly<Record<string, Readonly<Record<string, string>>>>;
export type MessageArgs = Readonly<Record<string, string | number>>;

export interface AppLocale {
  /** The language the SDK's dialogs and `t` use: `language` if set, else the best match for `navigator.languages`. */
  readonly resolved: string;
  /** `null` follows `navigator.languages`. */
  language: string | null;
  readonly available: readonly string[];
  /** `{ "de": { "app.hello": "Hallo {name}" } }`; `vn.*` keys reword or translate the SDK's own text. */
  define(messages: Messages): void;
  t(key: string, args?: MessageArgs): string;
}

interface LocaleState {
  language: string;
  override: string | null;
  available: string[];
}

declare const __vn_locale_state: (() => string) | undefined;
declare const __vn_locale_set_language: ((tag: string) => string) | undefined;
declare const __vn_locale_define: ((json: string) => string) | undefined;
declare const __vn_locale_t: ((key: string, args: string) => string) | undefined;

function stringArgs(args: MessageArgs | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(args ?? {})) out[k] = String(v);
  return out;
}

function fill(template: string, args: Record<string, string>): string {
  return template.replace(/\{([^{}]+)\}/g, (whole, name: string) => args[name] ?? whole);
}

function hostLocale(): AppLocale | undefined {
  if (
    typeof __vn_locale_state !== "function" ||
    typeof __vn_locale_set_language !== "function" ||
    typeof __vn_locale_define !== "function" ||
    typeof __vn_locale_t !== "function"
  )
    return undefined;
  const state = __vn_locale_state;
  const setLanguage = __vn_locale_set_language;
  const define = __vn_locale_define;
  const t = __vn_locale_t;
  const read = (): LocaleState => JSON.parse(state()) as LocaleState;
  return {
    get resolved() {
      return read().language;
    },
    get language() {
      return read().override;
    },
    set language(tag: string | null) {
      const error = setLanguage(tag ?? "");
      if (error) throw new RangeError(error);
    },
    get available() {
      return Object.freeze(read().available);
    },
    define(messages: Messages): void {
      const error = define(JSON.stringify(messages));
      if (error) throw new TypeError(error);
    },
    t(key: string, args?: MessageArgs): string {
      return t(key, JSON.stringify(stringArgs(args)));
    },
  };
}

function scriptLocale(): AppLocale {
  const catalogs = new Map<string, Map<string, string>>();
  let chosen: string | null = null;
  const preferred = (): string => chosen ?? (typeof navigator === "object" ? navigator?.language : undefined) ?? "en";
  const chain = (tag: string): string[] => {
    const parts = tag.split("-");
    const out = parts.map((_, i) => parts.slice(0, parts.length - i).join("-"));
    return [...out, "en"];
  };
  return {
    get resolved() {
      return chain(preferred()).find((t) => catalogs.has(t)) ?? "en";
    },
    get language() {
      return chosen;
    },
    set language(tag: string | null) {
      chosen = tag || null;
    },
    get available() {
      return Object.freeze([...catalogs.keys()]);
    },
    define(messages: Messages): void {
      for (const [tag, entries] of Object.entries(messages)) {
        const catalog = catalogs.get(tag) ?? new Map<string, string>();
        for (const [k, v] of Object.entries(entries)) catalog.set(k, v);
        catalogs.set(tag, catalog);
      }
    },
    t(key: string, args?: MessageArgs): string {
      for (const tag of chain(preferred())) {
        const text = catalogs.get(tag)?.get(key);
        if (text !== undefined) return fill(text, stringArgs(args));
      }
      return key;
    },
  };
}

export const appLocale: AppLocale = Object.freeze(hostLocale() ?? scriptLocale());
