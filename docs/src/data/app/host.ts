import { OK, all, importLine, section } from "./shared";

const IMPORT = importLine("appHost");
const PLATFORMS = all(OK);
const DEPRECATED = all("deprecated");
const NOT_EMBEDDED =
  "In an app run by its own executable (`sk dev`, `sk build`), there is no embedder: `available` is `false`, `functions` is empty, `call()` rejects and `on()` never fires.";
const INSTEAD =
  'Import the program\'s [native modules](/embedding/native-modules/): `import { search, on } from "native:notes"`, from a `#[skathi::module]` in Rust. Functions return their value, tasks a promise, events are typed, and `sk types` writes the declarations.';

export const host = section("Embedding", [
  {
    slug: "native-modules",
    name: 'import … from "native:<name>"',
    kind: "interface",
    summary: "Imports a module the program that embeds Skathi built in Rust: its functions, tasks and events.",
    platforms: PLATFORMS,
    since: "0.4.0",
    syntax: `import { available, on, off, search } from "native:notes";

export const available: boolean;
// The module's functions, tasks (they return a Promise) and:
export function on(event: string, handler: (data: unknown) => void): () => void;
export function off(event: string, handler: (data: unknown) => void): void;`,
    notes: [
      "A Rust program declares the module as an `impl` block with `#[skathi::module]` and hands a value to `Builder::module(name, value)`: each `pub fn` taking `&self` runs on the Skathi thread and returns its value, under its camelCase name; one marked `#[task]` runs on a worker and returns a promise; `events(name = Type)` declares what `Handle::emit_to` sends.",
      "Arguments and results are `#[skathi::data]` types, carried as serde values. An `Err` throws, or rejects for a task, with its message; a panic becomes an `Error`.",
      "An event arrives in the next frame after `emit_to`. A reload drops the handlers and rejects the tasks still waiting.",
      "`sk types --exe <program>` and `sk dev --exe` write `src/native-modules.d.ts` with the module's real signatures.",
      "Not embedded, the import still works: `available` is `false`, every function throws and `on` never fires.",
    ],
    examples: [
      {
        title: "Run both embedded and standalone",
        lang: "ts",
        code: `import { available, search, on } from "native:notes";

export async function find(query: string): Promise<string[]> {
  if (!available) return [];
  const notes = await search(query, { limit: 20 });
  return notes.map((n) => n.title);
}

on("progress", (p) => console.log(\`\${p.done} of \${p.total} synced\`));`,
      },
    ],
    related: ["app-host"],
    aliases: ["native module", "embed", "embedding", "invoke", "IPC", "host functions", "native:"],
  },
  {
    slug: "app-host",
    name: "appHost",
    kind: "property",
    summary: "The 0.3 way to call the embedding program's functions and receive its events. Use native modules.",
    platforms: DEPRECATED,
    syntax: `${IMPORT}

readonly appHost.available: boolean
readonly appHost.functions: readonly string[]
appHost.call<T = unknown>(name: string, ...args: unknown[]): Promise<T>
appHost.on(event: string, handler: (data: unknown) => void): () => void
appHost.off(event: string, handler: (data: unknown) => void): void`,
    notes: [
      "`appHost.call` reaches functions registered with `Builder::function`, and `appHost.on` events sent with `Handle::emit`; both are deprecated and print a `[skathi compat]` advisory once.",
      "`appHost.available` is not deprecated: it still says whether the app runs inside an embedding program.",
      NOT_EMBEDDED,
    ],
    workaround: INSTEAD,
    related: ["native-modules", "host-call", "host-on", "host-available"],
    aliases: ["appHost"],
  },
  {
    slug: "host-call",
    name: "appHost.call()",
    kind: "method",
    summary:
      "Calls a function the embedding program registered with `Builder::function`, with JSON arguments. Deprecated: use a native module.",
    platforms: DEPRECATED,
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
      'When the handler rejects, panics, or drops its reply, the promise rejects with an `AppHostError`: `name` is `"AppHostError"`, `message` is the embedder\'s text, and `function` the name called.',
    ],
    edgeCases: [
      "A name the embedder did not register rejects with a `TypeError` that lists the names it has.",
      "Not embedded: it rejects with an `AppHostError` and prints an advisory once.",
      'A reload rejects the waiting calls with "the page unloaded before the host answered".',
    ],
    workaround: INSTEAD,
    related: ["native-modules", "host-on", "app-host"],
    aliases: ["invoke", "AppHostError"],
  },
  {
    slug: "host-on",
    name: "appHost.on() / off()",
    kind: "method",
    summary: "Listens for an event sent with `Handle::emit`. Deprecated: use a native module's `on`.",
    platforms: DEPRECATED,
    syntax: `appHost.on(event: string, handler: (data: unknown) => void): () => void
appHost.off(event: string, handler: (data: unknown) => void): void`,
    returns: "`on()` returns a function that removes the handler, like `off()`.",
    notes: [
      "An event arrives in the app's next frame after `Handle::emit`. One sent while nothing listens is dropped, not queued.",
    ],
    edgeCases: ["Not embedded: handlers are kept and never called."],
    workaround: INSTEAD,
    related: ["native-modules", "host-call", "app-host"],
    aliases: ["listen", "emit", "host events"],
  },
  {
    slug: "host-available",
    name: "appHost.available / functions",
    kind: "property",
    summary: "Whether the app runs inside an embedding program, and the names of the functions it registered.",
    platforms: PLATFORMS,
    syntax: "readonly appHost.available: boolean\nreadonly appHost.functions: readonly string[]",
    notes: [
      "Each native module has an `available` export of its own, which is the check to use before calling it.",
      "`functions` lists only names registered with the deprecated `Builder::function`, sorted and frozen.",
    ],
    examples: [
      {
        title: "Show embedder-only settings",
        lang: "ts",
        code: `import { appHost } from "skathi:app";

export const embedded = appHost.available;`,
      },
    ],
    related: ["native-modules", "app-host"],
  },
]);
