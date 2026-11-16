import { OK, all, importLine, section } from "./shared";

const IMPORT = importLine("appHost");
const PLATFORMS = all(OK);
const NOT_EMBEDDED =
  "In an app run by its own executable (`sk dev`, `sk build`), there is no embedder: `available` is `false`, `functions` is empty, `call()` rejects and `on()` never fires.";

export const host = section("Embedding", [
  {
    slug: "app-host",
    name: "appHost",
    kind: "property",
    summary: "Calls async functions of the program that embeds Skathi as a library, and receives its events.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

readonly appHost.available: boolean
readonly appHost.functions: readonly string[]
appHost.call<T = unknown>(name: string, ...args: unknown[]): Promise<T>
appHost.on(event: string, handler: (data: unknown) => void): () => void
appHost.off(event: string, handler: (data: unknown) => void): void`,
    notes: [
      "A Rust program can run a Skathi app inside its own process with `sk_app::embed::Builder`. It registers functions with `Builder::function(name, handler)` and sends events with `Handle::emit(event, json)`; `appHost` is the app's side of both.",
      "Arguments and results cross as JSON: functions, `undefined` and class instances do not survive the trip.",
      "A reload rejects the calls still waiting and drops every handler.",
      NOT_EMBEDDED,
      "A Svelte app imports it from `@skathi-native/runtime/svelte`.",
    ],
    examples: [
      {
        title: "Run both embedded and standalone",
        lang: "ts",
        code: `import { appHost } from "@skathi-native/runtime";

export async function currentUser(): Promise<string> {
  if (appHost.available && appHost.functions.includes("user.name")) return appHost.call<string>("user.name");
  return "Guest";
}`,
      },
    ],
    limitations: [NOT_EMBEDDED],
    related: ["host-call", "host-on", "host-available"],
    aliases: ["embed", "embedding", "invoke", "IPC", "host functions"],
  },
  {
    slug: "host-call",
    name: "appHost.call()",
    kind: "method",
    summary:
      "Calls a function the embedding program registered, with JSON arguments, and resolves with its JSON result.",
    platforms: PLATFORMS,
    syntax: `appHost.call<T = unknown>(name: string, ...args: unknown[]): Promise<T>

interface AppHostError extends Error {
  readonly function: string;
}`,
    params: [
      { name: "name", type: "string", description: "The name given to `Builder::function`." },
      { name: "...args", type: "unknown[]", description: "Passed to the handler as one JSON array." },
    ],
    returns: "A promise for the parsed result. `T` is only a type assertion; nothing checks the shape.",
    notes: [
      "The handler runs on the Skathi thread, inside the call. One with slow work moves its reply to another thread and answers from there, so the UI does not wait.",
      'When the handler rejects, panics, or drops its reply, the promise rejects with an `AppHostError`: `name` is `"AppHostError"`, `message` is the embedder\'s text, and `function` the name called.',
    ],
    examples: [
      {
        title: "Ask the host to sign a request",
        lang: "ts",
        code: `import { appHost, type AppHostError } from "@skathi-native/runtime";

export async function sign(payload: { path: string; body: string }): Promise<string | null> {
  try {
    return await appHost.call<string>("auth.sign", payload);
  } catch (e) {
    console.warn(\`\${(e as AppHostError).function} failed:\`, e);
    return null;
  }
}`,
      },
    ],
    edgeCases: [
      "A name the embedder did not register rejects with a `TypeError` that lists the names it has.",
      "Arguments `JSON.stringify` cannot serialize (a cycle, a `BigInt`) reject with a `TypeError`.",
      "Not embedded: it rejects with an `AppHostError` and prints an advisory once.",
      'A reload rejects the waiting calls with "the page unloaded before the host answered".',
    ],
    related: ["host-available", "host-on", "app-host"],
    aliases: ["invoke", "AppHostError"],
  },
  {
    slug: "host-on",
    name: "appHost.on() / off()",
    kind: "method",
    summary: "Listens for an event the embedding program sends, with its JSON data.",
    platforms: PLATFORMS,
    syntax: `appHost.on(event: string, handler: (data: unknown) => void): () => void
appHost.off(event: string, handler: (data: unknown) => void): void`,
    returns: "`on()` returns a function that removes the handler, like `off()`.",
    notes: [
      "An event arrives in the app's next frame after `Handle::emit`. One sent while nothing listens is dropped, not queued.",
      "A handler that throws is logged; the other handlers still run.",
      "The same handler added twice for one event runs once.",
    ],
    examples: [
      {
        title: "Follow the host's online state",
        lang: "svelte",
        code: `<script lang="ts">
  import { appHost } from "@skathi-native/runtime/svelte";

  let online = $state(true);
  $effect(() => appHost.on("network", (data) => (online = (data as { online: boolean }).online)));
</script>

{#if !online}<p class="banner">Offline</p>{/if}`,
      },
    ],
    edgeCases: ["Not embedded: handlers are kept and never called."],
    related: ["host-call", "app-host"],
    aliases: ["listen", "emit", "host events"],
  },
  {
    slug: "host-available",
    name: "appHost.available / functions",
    kind: "property",
    summary: "Whether the app runs inside an embedding program, and the names of the functions it registered.",
    platforms: PLATFORMS,
    syntax: "readonly appHost.available: boolean\nreadonly appHost.functions: readonly string[]",
    notes: ["`functions` is sorted and frozen."],
    examples: [
      {
        title: "Hide a feature the host does not offer",
        lang: "ts",
        code: `import { appHost } from "@skathi-native/runtime";

export const canPrintNatively = appHost.functions.includes("print.native");`,
      },
    ],
    related: ["host-call", "app-host"],
  },
]);
