import { OK, importLine, section } from "./shared";

export const toml = section("skathi:toml", [
  {
    slug: "toml",
    name: "TOML",
    kind: "namespace",
    summary: "Parses and writes TOML 1.1 documents, in the shape of `JSON`.",
    syntax: `${importLine("skathi:toml", "TOML")}

TOML.parse(text: string): Record<string, unknown>
TOML.stringify(value: Record<string, unknown>, options?: { pretty?: boolean }): string`,
    params: [
      { name: "text", type: "string", description: "`parse`: a TOML document." },
      {
        name: "value",
        type: "Record<string, unknown>",
        description:
          "`stringify`: the object to write. A TOML document is a table, so the top level must be an object.",
      },
      {
        name: "options.pretty",
        type: "boolean",
        description: "`stringify`: put each array element on its own line.",
        optional: true,
        default: "false",
      },
    ],
    returns: "`parse` returns the document as an object. `stringify` returns the document as a string.",
    values: [
      { value: "String", status: OK, note: "`string`." },
      { value: "Boolean", status: OK, note: "`boolean`." },
      { value: "Float", status: OK, note: "`number`." },
      {
        value: "Integer",
        status: OK,
        note: "`number`. A `bigint` when it is beyond the safe integers, from ±2^53 on.",
      },
      { value: "Offset date-time", status: OK, note: "`Date`." },
      { value: "Local date-time", status: OK, note: "`Temporal.PlainDateTime`." },
      { value: "Local date", status: OK, note: "`Temporal.PlainDate`." },
      { value: "Local time", status: OK, note: "`Temporal.PlainTime`." },
      { value: "Array", status: OK, note: "`Array`." },
      { value: "Table", status: OK, note: "A plain object." },
      { value: "Array of tables", status: OK, note: "An `Array` of objects." },
    ],
    notes: [
      "The table above is how `TOML.parse` maps each TOML type to a JavaScript value.",
      "`TOML.parse` keeps the keys in document order.",
      "`TOML.stringify` leaves `undefined`, functions and symbols out of a table, as `JSON.stringify` leaves them out of an object.",
      "`TOML.stringify` writes a whole `number` within the safe integers as an integer, and any other `number` as a float.",
      "A `bigint` is written as an integer.",
      "A `Date` is written as an offset date-time in UTC: `new Date(0)` becomes `1970-01-01T00:00:00.0Z`. `Temporal.PlainDateTime`, `PlainDate` and `PlainTime` are written as the local forms.",
      "A nested object is written as a table, an array of objects as an array of tables. Any other object is written as a table of its own enumerable keys.",
      "An integer beyond the safe integers survives `parse`, `stringify` and `parse` again as the same `bigint`.",
      'An app can import a `.toml` file: `import config from "./config.toml"`. `sk dev`, `sk build` and a build with a Vite config all treat it the same way.',
      "The default export is `TOML.parse` of the file's text, run when the module loads, so it holds the same values as the table above: `bigint`, `Date` and `Temporal` included. There are no named exports. TypeScript types it as `Record<string, unknown>`.",
      "Invalid TOML in an imported file fails the build with the file, line and column. A Vite config run by Node cannot check it at build time, so the import throws the `SyntaxError` when the module loads.",
      "In `sk dev`, saving an imported `.toml` file under `src/` reloads the modules that import it.",
    ],
    examples: [
      {
        title: "Read a config file's text",
        lang: "ts",
        code: `import { TOML } from "skathi:toml";

const text = \`title = "Skathi"
big = 9007199254740993
when = 1979-05-27T07:32:00Z
day = 1979-05-27
tags = ["a", "b"]

[window]
width = 800
resizable = true

[[servers]]
name = "one"
\`;

const config = TOML.parse(text);
Object.keys(config); // ["title", "big", "when", "day", "tags", "window", "servers"]
config.big; // 9007199254740993n
config.when; // Date 1979-05-27T07:32:00.000Z
config.day; // Temporal.PlainDate 1979-05-27
config.window; // { width: 800, resizable: true }
config.servers; // [{ name: "one" }]`,
      },
      {
        title: "Import a config file",
        lang: "ts",
        code: `// src/config.toml:
// title = "Skathi"
// [window]
// width = 800

import config from "./config.toml";

const window = config.window as { width: number };
window.width; // 800`,
      },
      {
        title: "Write an object as a document",
        lang: "ts",
        code: `import { TOML } from "skathi:toml";

const text = TOML.stringify({
  name: "x",
  skipped: undefined,
  n: 1n << 62n,
  at: new Date(0),
  list: [1, 2],
  t: { ok: true },
});
// name = "x"
// n = 4611686018427387904
// at = 1970-01-01T00:00:00.0Z
// list = [1, 2]
//
// [t]
// ok = true`,
      },
      {
        title: "Report a syntax error to the user",
        lang: "ts",
        code: `import { TOML } from "skathi:toml";

export function readSettings(text: string): Record<string, unknown> | string {
  try {
    return TOML.parse(text);
  } catch (error) {
    if (error instanceof SyntaxError) return error.message;
    throw error;
  }
}

readSettings('a = 1\\nb = "unclosed\\n'); // "TOML.parse: line 2, column 14: …"`,
      },
    ],
    edgeCases: [
      "`parse`: invalid TOML throws a `SyntaxError` with the position, counted from 1: `TOML.parse: line 4, column 9: expected …`.",
      "`parse`: a `text` that is not a string throws a `TypeError`.",
      "`stringify`: `null` throws a `TypeError` that names the key, since TOML has no null: `TOML.stringify: window.title is null, which TOML cannot represent`.",
      "`stringify`: `undefined`, a function or a symbol in an array throws a `TypeError` that names the element: `TOML.stringify: list[1] is undefined or a function, which TOML cannot represent`.",
      "`stringify`: a `bigint` outside 64 bits throws a `RangeError`.",
      "`stringify`: a top-level value that is not an object, such as an array or a string, throws a `TypeError`.",
      "`stringify`: an object that contains itself, or nesting deeper than 128 levels, throws a `TypeError`.",
    ],
    related: ["integer-types-64"],
    aliases: [
      "TOML.parse",
      "TOML.stringify",
      "toml parser",
      "config file",
      "Cargo.toml",
      "pyproject.toml",
      "Temporal.PlainDate",
      "pretty",
      "import .toml",
      "config.toml",
    ],
  },
]);
