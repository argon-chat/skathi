import { OK, all, importLine, section } from "./shared";

const IMPORT = importLine("appLocale");
const PLATFORMS = all(OK);
const BUILT_IN =
  "`am`, `cs`, `da`, `de`, `el`, `en`, `es`, `fi`, `fr`, `hi`, `hu`, `id`, `it`, `ja`, `kk`, `ko`, `nb`, `nl`, `pl`, `pt-BR`, `ro`, `ru`, `sv`, `tr`, `uz`, `vi`, `zh-Hans` and `zh-Hant`";
const NOT_REACTIVE =
  "Not reactive: changing `language` does not re-render anything by itself. Keep the language in a `ref` (Vue) or `$state` (Svelte) and read `t` through it.";

export const locale = section("Locale", [
  {
    slug: "app-locale",
    name: "appLocale",
    kind: "property",
    summary:
      "Picks the language for the SDK's own dialogs and the app's strings, and looks up the app's translated text with `{name}` placeholders.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

readonly appLocale.resolved: string
appLocale.language: string | null
readonly appLocale.available: readonly string[]
appLocale.define(messages: Messages): void
appLocale.t(key: string, args?: MessageArgs): string`,
    notes: [
      `The SDK's own text (the microphone prompt, the crash dialog) ships in ${BUILT_IN}. An app adds its own catalogs with \`define()\`, and \`sk.*\` keys reword or translate the SDK's text.`,
      "The language follows `navigator.languages` unless `language` overrides it. `navigator.language` itself never changes.",
      "Catalogs and the override live in the host process: they outlast a reload, and are gone at the next launch.",
      NOT_REACTIVE,
    ],
    examples: [
      {
        title: "A small i18n composable",
        lang: "ts",
        code: `import { computed, ref } from "vue";
import type { MessageArgs } from "@skathi-native/runtime";
import { appLocale } from "skathi:app";

appLocale.define({
  en: { "inbox.title": "Inbox", "inbox.unread": "{count} unread" },
  de: { "inbox.title": "Posteingang", "inbox.unread": "{count} ungelesen" },
});

const language = ref(appLocale.resolved);

export function useI18n() {
  const setLanguage = (tag: string | null) => {
    appLocale.language = tag;
    language.value = appLocale.resolved;
  };
  const t = computed(() => {
    void language.value;
    return (key: string, args?: MessageArgs) => appLocale.t(key, args);
  });
  return { language, setLanguage, t };
}`,
        note: "Templates call `t.value(…)` through the computed, so they re-render when `setLanguage()` runs.",
      },
    ],
    related: ["locale-define", "locale-t", "locale-language", "locale-resolved"],
    aliases: ["i18n", "translations", "localization", "app.getLocale"],
  },
  {
    slug: "locale-define",
    name: "appLocale.define()",
    kind: "method",
    summary: "Adds translated strings for one or more languages, merged into what was defined before.",
    platforms: PLATFORMS,
    syntax: `appLocale.define(messages: Messages): void

type Messages = Readonly<Record<string, Readonly<Record<string, string>>>>; // { tag: { key: text } }`,
    params: [
      {
        name: "messages",
        type: "Messages",
        description:
          "Language tags (`en`, `pt-BR`, `zh-Hant`) mapping keys to text. Text may hold `{name}` placeholders.",
      },
    ],
    returns: "`undefined`.",
    notes: [
      "Keys are merged per language: a key defined again replaces the old text, other keys stay.",
      "A tag defined here joins `appLocale.available` and can be chosen by the language negotiation.",
      'Keys starting with `sk.` replace the SDK\'s own text in that language, for example `"sk.consent.allow": "&Erlauben"` in German. `&` marks the access key in dialog buttons.',
      "Tags are matched case-insensitively and stored in canonical form.",
    ],
    examples: [
      {
        title: "Load catalogs shipped with the app",
        lang: "ts",
        code: `import type { Messages } from "@skathi-native/runtime";
import { appLocale } from "skathi:app";
import en from "./locales/en.json";
import fr from "./locales/fr.json";

appLocale.define({ en, fr } satisfies Messages);`,
      },
      {
        title: "Reword the SDK's microphone prompt",
        lang: "ts",
        code: `import { appLocale } from "skathi:app";

appLocale.define({
  en: { "sk.consent.microphone.title": "Let {app} hear you in calls?" },
});`,
        note: "The SDK keys and their placeholders are `sk.app.this_app`, `sk.consent.microphone.title` and `.body` (`{app}`), `sk.consent.camera.title` and `.body`, `sk.consent.allow`, `sk.consent.block`, and the `sk.crash.*` keys of the crash dialog.",
      },
    ],
    edgeCases: [
      "A tag that is not a BCP 47 language tag, or a value that is not an object of strings, throws a `TypeError`; nothing from that call is defined.",
    ],
    related: ["locale-t", "locale-resolved", "app-locale"],
    aliases: ["Messages", "catalog", "translations"],
  },
  {
    slug: "locale-t",
    name: "appLocale.t()",
    kind: "method",
    summary: "Returns the text of a key in the resolved language, with `{name}` placeholders filled in.",
    platforms: PLATFORMS,
    syntax: `appLocale.t(key: string, args?: MessageArgs): string

type MessageArgs = Readonly<Record<string, string | number>>;`,
    params: [
      { name: "key", type: "string", description: "A key given to `define()`, or an `sk.*` key." },
      {
        name: "args",
        type: "MessageArgs",
        description: "Values for the placeholders; numbers are turned into text.",
        optional: true,
      },
    ],
    returns: "The text. A key found nowhere returns the key itself.",
    notes: [
      "Lookup: the resolved language, then shorter forms of it (`de-AT`, then `de`), then `en`. At each step the app's catalog comes before the SDK's.",
      "`{name}` is replaced by `args.name`. A placeholder without a matching argument is left as written.",
      "There are no plural rules or number formatting; choose the key in code, and format with `Intl` before passing the value.",
    ],
    examples: [
      {
        title: "Plural by key",
        lang: "ts",
        code: `import { appLocale } from "skathi:app";

export function unread(count: number): string {
  const key = count === 1 ? "inbox.unread.one" : "inbox.unread.other";
  return appLocale.t(key, { count: count.toLocaleString(appLocale.resolved) });
}`,
      },
    ],
    edgeCases: [
      "A language picked from the SDK's own list may have no catalog of the app's: then `t` falls back to the app's `en` text, while the SDK's dialogs are still translated.",
    ],
    related: ["locale-define", "locale-resolved"],
    aliases: ["translate", "MessageArgs", "i18n.t"],
  },
  {
    slug: "locale-language",
    name: "appLocale.language",
    kind: "property",
    summary: "The language the user chose in the app, overriding `navigator.languages`; `null` follows the system.",
    platforms: PLATFORMS,
    syntax: "appLocale.language: string | null",
    notes: [
      "Setting it changes `resolved`, the text of `t`, and the language of the SDK's dialogs from then on.",
      "`null`, `undefined` or an empty string go back to following the system. A tag is stored in canonical form.",
      "It is not saved across launches: keep the choice in `localStorage` and set it again at startup.",
      NOT_REACTIVE,
    ],
    examples: [
      {
        title: "A language menu in Svelte",
        lang: "svelte",
        code: `<script lang="ts">
  import { appLocale } from "skathi:app";

  appLocale.language = localStorage.getItem("language");
  let current = $state(appLocale.resolved);

  function choose(tag: string | null) {
    appLocale.language = tag;
    if (tag) localStorage.setItem("language", tag);
    else localStorage.removeItem("language");
    current = appLocale.resolved;
  }
</script>

<select value={appLocale.language ?? ""} onchange={(e) => choose(e.currentTarget.value || null)}>
  <option value="">System ({current})</option>
  <option value="en">English</option>
  <option value="de">Deutsch</option>
</select>`,
      },
    ],
    edgeCases: ["A value that is not a language tag throws a `RangeError` and keeps the previous setting."],
    related: ["locale-resolved", "app-locale"],
    aliases: ["setLocale", "language override"],
  },
  {
    slug: "locale-resolved",
    name: "appLocale.resolved / available",
    kind: "property",
    summary: "The language actually in use after negotiation, and every language there is text for.",
    platforms: PLATFORMS,
    syntax: "readonly appLocale.resolved: string\nreadonly appLocale.available: readonly string[]",
    notes: [
      "`available` is the SDK's built-in languages followed by the ones the app defined, without duplicates.",
      "`resolved` is the first preference (`language`, then `navigator.languages`) that matches an available language exactly or by a shorter form, else another variant of the same language (`pt-PT` gets `pt-BR`), else `en`.",
      "Chinese without a script is mapped first: `zh-TW`, `zh-HK` and `zh-MO` to `zh-Hant`, other `zh` to `zh-Hans`.",
      "Both are read fresh on each access.",
    ],
    examples: [
      {
        title: "Format dates in the app's language",
        lang: "ts",
        code: `import { appLocale } from "skathi:app";

export const formatDay = (date: Date) =>
  new Intl.DateTimeFormat(appLocale.resolved, { weekday: "long", day: "numeric", month: "long" }).format(date);`,
      },
    ],
    related: ["locale-language", "locale-t", "locale-define"],
    aliases: ["app.getLocale", "negotiation"],
  },
]);
