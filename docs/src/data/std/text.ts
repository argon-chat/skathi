import { importLine, section } from "./shared";

const UNITS =
  "Lengths and positions are in UTF-16 code units, as `string.length` and `indexOf()` are, so they can be passed to `slice()`.";
const NOT_YET =
  "`StringComparer`, `StringTokenizer` and `Unicode` (collation, word and sentence boundaries, normalization and character properties) are not in this module yet. Use `Intl.Collator`, `Intl.Segmenter` and `String.prototype.normalize()` until they are.";

export const text = section("skathi:text", [
  {
    slug: "string-builder",
    name: "StringBuilder",
    kind: "class",
    summary: "Text put together from many parts in one buffer the host keeps.",
    syntax: `${importLine("skathi:text", "StringBuilder")}

new StringBuilder(initial?: string)

readonly builder.length: number
builder.append(value: unknown): this
builder.appendLine(value?: unknown): this
builder.appendAll(parts: readonly unknown[], separator?: string): this
builder.insert(index: number, text: string): this
builder.clear(): void
builder.toString(): string`,
    params: [{ name: "initial", type: "string", description: "The text to start from. Empty by default." }],
    notes: [
      "`append(value)` adds `String(value)`, as `+=` would: a number, `null` or an object with `toString()` is converted.",
      "`appendLine(value)` adds the value, when one is given, and then `\\n`.",
      "`appendAll(parts, separator)` adds every element of an array in one call into the host, with the separator between them. This is the method to use in a loop of thousands of parts: one call instead of one per part.",
      "`insert(index, text)` puts text at a position from 0 to `length`.",
      "For a handful of concatenations `+=` and a template literal are as fast: the engine joins strings lazily. A builder pays off when the parts are many, when text is inserted in the middle, or when the same buffer is filled and cleared again and again.",
      UNITS,
      NOT_YET,
    ],
    edgeCases: [
      "`RangeError`: `insert()` at an index past `length`, or a text that would be longer than a string can be (2^31 - 1 code units).",
      "`TypeError`: `initial`, the separator or the text given to `insert()` is not a string.",
    ],
    examples: [
      {
        title: "Build a CSV file",
        lang: "ts",
        code: `import { StringBuilder } from "skathi:text";

const csv = new StringBuilder();
csv.appendAll(["id", "name", "total"], ",").appendLine();
for (const row of rows) {
  csv.appendAll([row.id, row.name, row.total], ",").appendLine();
}
csv.length; // in UTF-16 code units
const file = csv.toString();`,
      },
    ],
    related: ["string-writer", "string-reader"],
    aliases: ["string buffer", "concat", "join", "append", "appendAll", "appendLine"],
  },
  {
    slug: "string-writer",
    name: "StringWriter",
    kind: "class",
    summary: "A sink for text: write, write a line, and take the whole text at the end.",
    syntax: `${importLine("skathi:text", "StringWriter")}

new StringWriter()

readonly writer.length: number
writer.write(value: unknown): void
writer.writeLine(value?: unknown): void
writer.writeAll(parts: readonly unknown[], separator?: string): void
writer.toString(): string`,
    notes: [
      "The same buffer as a [`StringBuilder`](/std/string-builder/) under the names an output has. Take one as a parameter in code that prints (a report, a serializer, generated source), so that the caller decides where the text goes.",
      "`write(value)` adds `String(value)`. `writeLine(value)` adds the value, when one is given, and `\\n`. `writeAll(parts, separator)` adds every element of an array in one call.",
      UNITS,
    ],
    edgeCases: ["`RangeError`: the text would be longer than a string can be (2^31 - 1 code units)."],
    examples: [
      {
        title: "A function that prints into whatever it is given",
        lang: "ts",
        code: `import { StringWriter } from "skathi:text";

function printTree(node: TreeNode, out: StringWriter, depth = 0) {
  out.writeLine("  ".repeat(depth) + node.name);
  for (const child of node.children) printTree(child, out, depth + 1);
}

const out = new StringWriter();
printTree(root, out);
console.log(out.toString());`,
      },
    ],
    related: ["string-builder", "string-reader"],
    aliases: ["write", "writeLine", "writeAll", "text writer", "output"],
  },
  {
    slug: "string-reader",
    name: "StringReader",
    kind: "class",
    summary: "A position in a string that moves forward by characters or by lines, without splitting the string first.",
    syntax: `${importLine("skathi:text", "StringReader")}

new StringReader(text: string)

readonly reader.position: number
readonly reader.length: number
reader.read(): string | undefined
reader.peek(): string | undefined
reader.readLine(): string | null
reader.readLines(max?: number): string[]
reader.readToEnd(): string`,
    params: [{ name: "text", type: "string", description: "The text to read. The reader keeps its own copy." }],
    notes: [
      "`read()` returns the next character and moves past it. A character outside the Basic Multilingual Plane, an emoji for example, comes as one string of two code units. `peek()` returns the same without moving. Both return `undefined` at the end.",
      '`readLine()` returns the next line without its ending, which may be `\\n`, `\\r\\n` or `\\r`, and `null` at the end. An empty line is `""`. A text that ends with a line ending does not have one more empty line after it.',
      '`readLines(max)` returns every line that is left, or at most `max` of them, in one call. For a large text read in portions this avoids making one array of all its lines, which `split("\\n")` does.',
      "`readToEnd()` returns what is left and moves to the end.",
      "`position` is where the next read starts. " + UNITS,
    ],
    edgeCases: ["`TypeError`: `text` is not a string.", "`RangeError`: `max` is not a non-negative integer."],
    examples: [
      {
        title: "Read a header, then the rest in portions",
        lang: "ts",
        code: `import { StringReader } from "skathi:text";

const reader = new StringReader(log);
const header = reader.readLine(); // "# exported 2026-11-21"

for (let lines = reader.readLines(500); lines.length; lines = reader.readLines(500)) {
  importBatch(lines);
  await nextFrame(); // the window keeps painting between portions
}`,
      },
    ],
    related: ["string-builder", "string-writer"],
    aliases: ["readLine", "readLines", "lines", "split lines", "text reader", "peek"],
  },
  {
    slug: "text-diff",
    name: "TextDiff",
    kind: "namespace",
    summary: "The shortest set of changes that turns one text into another, by lines or by characters.",
    syntax: `${importLine("skathi:text", "TextDiff")}

TextDiff.lines(oldText: string, newText: string, options?: { context?: number }): DiffHunk[]
TextDiff.chars(oldText: string, newText: string): DiffOp[]
TextDiff.unified(oldText: string, newText: string, options?: {
  context?: number
  oldName?: string
  newName?: string
}): string

interface DiffOp { op: "equal" | "delete" | "insert"; text: string }
interface DiffHunk {
  oldStart: number; oldLines: number   // line numbers start at 1
  newStart: number; newLines: number
  lines: DiffOp[]                       // one per line, without its "\\n"
}`,
    params: [
      { name: "oldText", type: "string", description: "The text before the change." },
      { name: "newText", type: "string", description: "The text after it." },
      {
        name: "options.context",
        type: "number",
        description: "How many unchanged lines to keep around a change. 3 by default.",
      },
      {
        name: "options.oldName, options.newName",
        type: "string",
        description: "For `unified()`: with both, the text starts with the `---` and `+++` lines.",
      },
    ],
    notes: [
      "`lines()` returns hunks: each is a group of changed lines with `context` unchanged lines around it, and two changes closer than that share a hunk. Equal texts give `[]`.",
      "`chars()` returns runs of characters in order. Joining the `equal` and `insert` runs gives the new text; joining `equal` and `delete` gives the old one. A surrogate pair is never split.",
      '`unified()` returns the line changes as `diff -u` prints them, ready for `git apply` when the names are given. Equal texts give `""`.',
      "The result has the fewest deletions and insertions possible (Myers' algorithm, in linear memory). The time grows with the size of the texts multiplied by the number of differences: two versions of a file compare quickly, two unrelated megabyte texts do not.",
      "A line is what ends with `\\n`. `\\r\\n` is not treated specially, so a file whose line endings changed differs in every line. A last line without `\\n` differs from the same line with it, and `unified()` marks it with `\\ No newline at end of file`.",
      "The call is synchronous. Compare very large texts off the first frame, after the window is shown.",
    ],
    edgeCases: [
      "`TypeError`: a text is not a string, or only one of `oldName` and `newName` is given.",
      "`RangeError`: `context` is not a non-negative integer.",
    ],
    examples: [
      {
        title: "Show what changed in a document",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

const hunks = TextDiff.lines(saved, edited, { context: 2 });
for (const hunk of hunks) {
  console.log(\`line \${hunk.oldStart}:\`);
  for (const line of hunk.lines) {
    const mark = line.op === "insert" ? "+" : line.op === "delete" ? "-" : " ";
    console.log(mark, line.text);
  }
}`,
      },
      {
        title: "Highlight the edit inside one line",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

TextDiff.chars("color: red;", "color: darkred;");
// [
//   { op: "equal", text: "color: " },
//   { op: "insert", text: "dark" },
//   { op: "equal", text: "red;" },
// ]`,
      },
      {
        title: "A patch file",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

const patch = TextDiff.unified("one\\ntwo\\n", "one\\n2\\n", { oldName: "a/notes.txt", newName: "b/notes.txt" });
// --- a/notes.txt
// +++ b/notes.txt
// @@ -1,2 +1,2 @@
//  one
// -two
// +2`,
      },
    ],
    related: ["text-search", "string-reader"],
    aliases: ["diff", "patch", "unified diff", "Myers", "compare texts", "DiffHunk", "DiffOp", "changes"],
  },
  {
    slug: "text-search",
    name: "TextSearch",
    kind: "class",
    summary: "Many fixed strings looked for in one pass over a text.",
    syntax: `${importLine("skathi:text", "TextSearch")}

new TextSearch(patterns: readonly string[], options?: { ignoreCase?: boolean; wholeWord?: boolean })

search.findAll(text: string): { index: number; length: number; pattern: number }[]
search.test(text: string): boolean`,
    params: [
      {
        name: "patterns",
        type: "string[]",
        description: "The strings to look for, taken literally. None may be empty. An empty list matches nothing.",
      },
      {
        name: "options.ignoreCase",
        type: "boolean",
        description: "Compare without regard to case, for any script: `КОТ` finds `кот`.",
      },
      {
        name: "options.wholeWord",
        type: "boolean",
        description: "Accept a match only when no letter, digit or `_` touches it on either side.",
      },
    ],
    notes: [
      "The patterns are compiled once into one automaton (Aho-Corasick), and a search reads the text once however many patterns there are. Make the search once and keep it. For a single pattern, or for a pattern with syntax, a `RegExp` is the tool.",
      '`findAll()` returns the matches from left to right. No match overlaps another. Of the patterns that match at the same place, the longest wins: with `["cat", "category"]` the text `category` is one match of the second pattern.',
      "`pattern` is the position of the pattern in the list given to the constructor.",
      "`index` and `length` are in UTF-16 code units, so `text.slice(m.index, m.index + m.length)` is the matched text.",
      "`ignoreCase` lowers each character on its own, without a locale. Characters whose lower case is more than one character (`İ`) are compared as they are, and `ß` does not match `ss`.",
    ],
    edgeCases: [
      "`TypeError`: `patterns` is not an array, a pattern is not a string, or a pattern is empty.",
      "`TypeError`: the text is not a string.",
    ],
    examples: [
      {
        title: "Highlight every mention",
        lang: "ts",
        code: `import { TextSearch } from "skathi:text";

const names = ["Ada", "Grace", "Linus"];
const mentions = new TextSearch(names, { ignoreCase: true, wholeWord: true });

for (const m of mentions.findAll(message)) {
  highlight(m.index, m.length, names[m.pattern]);
}`,
      },
      {
        title: "A word filter",
        lang: "ts",
        code: `import { TextSearch } from "skathi:text";

const blocked = new TextSearch(blockedWords, { ignoreCase: true });
if (blocked.test(input.value)) showWarning();`,
      },
    ],
    related: ["glob", "text-diff"],
    aliases: ["Aho-Corasick", "multi-pattern", "find all", "keywords", "highlight", "word filter", "indexOf many"],
  },
  {
    slug: "glob",
    name: "Glob",
    kind: "class",
    summary: "A path pattern such as src/**/*.ts, matched against strings.",
    syntax: `${importLine("skathi:text", "Glob")}

new Glob(pattern: string, options?: { caseSensitive?: boolean })

glob.match(path: string): boolean
glob.filter(paths: readonly string[]): string[]`,
    params: [
      { name: "pattern", type: "string", description: "The pattern, in the syntax below." },
      {
        name: "options.caseSensitive",
        type: "boolean",
        description: "`true` by default. `false` compares without regard to case.",
      },
    ],
    values: [
      { value: "*", status: "supported", note: "Any characters except `/`, or none." },
      {
        value: "**",
        status: "supported",
        note: "As a whole path segment: any number of segments, or none. `src/**/*.ts` matches `src/a.ts` and `src/x/y/a.ts`. Inside a segment (`a**b`) it is `*`.",
      },
      { value: "?", status: "supported", note: "One character except `/`." },
      {
        value: "[abc] [a-z] [!a-z]",
        status: "supported",
        note: "One character of a set or a range; `!` or `^` first negates it. It never matches `/`.",
      },
      {
        value: "{a,b}",
        status: "supported",
        note: "Either alternative; they nest. At most 1024 patterns after expansion.",
      },
      { value: "\\", status: "supported", note: "Takes the next character literally: `\\*` is a star." },
      { value: "!pattern", status: "supported", note: "A leading `!` negates the whole pattern." },
    ],
    notes: [
      "The syntax is the one `Bun.Glob` and shell globs use. The whole path must match, not a part of it.",
      "`match()` only compares strings: it does not read a folder. To list files, get the paths first and pass them to `filter()`, which checks them all in one call.",
      "The separator is `/`. On Windows a `\\` in the path counts as `/`, so paths from the system match the same patterns.",
      "A name that starts with a dot is matched like any other: `*` matches `.env`.",
      "An unclosed `[` or `{` is an ordinary character.",
    ],
    edgeCases: [
      "`TypeError`: the pattern or a path is not a string, or `paths` is not an array.",
      "`RangeError`: the braces expand to more than 1024 patterns.",
    ],
    examples: [
      {
        title: "Pick the files a rule applies to",
        lang: "ts",
        code: `import { Glob } from "skathi:text";

const sources = new Glob("src/**/*.{ts,vue}");
sources.match("src/components/App.vue"); // true
sources.match("src/main.ts"); // true
sources.match("docs/main.ts"); // false

const notDocs = new Glob("!**/*.md");
notDocs.filter(["README.md", "src/a.ts", "docs/b.md"]); // ["src/a.ts"]`,
      },
    ],
    related: ["text-search"],
    aliases: ["wildcard", "pattern", "minimatch", "Bun.Glob", "globstar", "file pattern", "match path"],
  },
]);
