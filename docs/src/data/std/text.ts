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
    summary:
      "The changes that turn one text into another, by lines, words or characters, and a three-way merge with conflict markers.",
    syntax: `${importLine("skathi:text", "TextDiff")}

type DiffSource = string | Uint8Array | NativeBuffer   // bytes are read as UTF-8

TextDiff.changes(oldText: DiffSource, newText: DiffSource, options?: DiffOptions): DiffChange[]
TextDiff.changesAsync(oldText: DiffSource, newText: DiffSource, options?: DiffOptions): Promise<DiffChange[]>
TextDiff.lines(oldText: DiffSource, newText: DiffSource, options?: DiffOptions): DiffHunk[]
TextDiff.words(oldText: string, newText: string, options?: DiffOptions): DiffOp[]
TextDiff.chars(oldText: string, newText: string, options?: DiffOptions): DiffOp[]
TextDiff.unified(oldText: DiffSource, newText: DiffSource, options?: DiffOptions & {
  oldName?: string
  newName?: string
}): string
TextDiff.merge(base: string, ours: string, theirs: string, options?: {
  style?: "merge" | "diff3" | "zdiff3"
  labels?: [base: string, ours: string, theirs: string]
}): { text: string; conflicts: MergeConflict[] }

interface DiffOptions {
  algorithm?: "histogram" | "myers" | "patience"
  ignoreWhitespace?: "none" | "trailing" | "amount" | "all"
  ignoreCase?: boolean
  ignoreLineEndings?: boolean
  context?: number
  inline?: "none" | "words" | "chars"
}
interface DiffChange {
  oldStart: number; oldEnd: number      // lines from 0, the end not included
  newStart: number; newEnd: number
  inline?: { oldFrom: number; oldTo: number; newFrom: number; newTo: number }[]
}
interface DiffOp { op: "equal" | "delete" | "insert"; text: string }
interface DiffHunk {
  oldStart: number; oldLines: number    // line numbers start at 1
  newStart: number; newLines: number
  lines: DiffOp[]                       // one per line, without its "\\n"
}
interface MergeConflict {
  base: [number, number]; ours: [number, number]; theirs: [number, number]
  at: number                            // the line of "<<<<<<<" in text
}`,
    params: [
      { name: "oldText", type: "DiffSource", description: "The text before the change." },
      { name: "newText", type: "DiffSource", description: "The text after it." },
      {
        name: "options.algorithm",
        type: "string",
        description:
          '`"histogram"` by default: on code it gives the hunks a person would write and does not pair up unrelated closing braces. `"myers"` gives the fewest deletions and insertions; `chars()` uses it by default. `"patience"` anchors on lines that occur once on each side.',
      },
      {
        name: "options.ignoreWhitespace",
        type: "string",
        description:
          '`"trailing"` ignores spaces at the end of a line, `"amount"` also treats any run of spaces as one, `"all"` ignores every space and tab. Like `git diff` with `--ignore-space-at-eol`, `-b` and `-w`.',
      },
      { name: "options.ignoreCase", type: "boolean", description: "Compare without regard to case, for any script." },
      {
        name: "options.ignoreLineEndings",
        type: "boolean",
        description: "Treat `\\r\\n` and `\\n` as the same line ending.",
      },
      {
        name: "options.context",
        type: "number",
        description: "For `lines()`, `unified()` and patches: unchanged lines kept around a change. 3 by default.",
      },
      {
        name: "options.inline",
        type: "string",
        description:
          'For `changes()`: `"words"` or `"chars"` also compares the replaced lines inside, for the highlight GitHub and editors show within a line.',
      },
      {
        name: "options.oldName, options.newName",
        type: "string",
        description: "For `unified()`: with both, the text starts with the `---` and `+++` lines.",
      },
      {
        name: "options.style",
        type: "string",
        description:
          'For `merge()`: `"merge"` shows both sides in a conflict, `"diff3"` adds the base lines between them, `"zdiff3"` does too and moves lines both sides share out of the conflict, as `"merge"` does.',
      },
      {
        name: "options.labels",
        type: "[string, string, string]",
        description:
          'For `merge()`: the names after the markers, in the order of the arguments: base, ours, theirs. `["base", "ours", "theirs"]` by default.',
      },
    ],
    notes: [
      "`changes()` is the call for large files: it returns four numbers per change and copies no text into JS. Slice the lines yourself, or show them from the text you already have. Equal texts give `[]`.",
      "`changesAsync()` does the same on a worker thread and answers with a promise, so a diff of tens of thousands of lines does not hold a frame.",
      "`inline` offsets are in UTF-16 code units from the start of each whole text, so `oldText.slice(oldFrom, oldTo)` is the changed part. A change that only adds or only removes lines has an empty `inline`, and so does one longer than 16 384 code units on a side (4096 for `chars`).",
      "`lines()` returns hunks: each is a group of changed lines with `context` unchanged lines around them, and two changes closer than that share a hunk.",
      "`words()` compares runs of letters and digits (and `_`), runs of spaces, line breaks and every other character on its own; an emoji with its modifiers or joiners stays one piece. `chars()` compares characters. In both, joining the `equal` and `insert` runs gives the new text and joining `equal` and `delete` gives the old one.",
      '`unified()` returns the line changes as `diff -u` prints them. Equal texts give `""`. For a patch you go on to apply, cut down or reverse, use [`Patch.from()`](/std/patch/).',
      "Bytes are read as UTF-8 in the host, without making a string in JS first: a file read into a `NativeBuffer` is compared as it is. Bytes that are not UTF-8 count as U+FFFD.",
      "A line is what ends with `\\n`; a `\\r` before it is part of the line unless `ignoreLineEndings` is set. A last line without `\\n` differs from the same line with it, and `unified()` marks it with `\\ No newline at end of file`.",
      "`merge()` applies the changes from `base` to `ours` and from `base` to `theirs`. Changes to different lines merge; changes that overlap or touch conflict unless both sides made the same one, as in git. `conflicts` gives each conflict's lines in the three inputs and where its `<<<<<<<` line is in `text`, so a conflict screen does not have to parse the markers. Markers end in `\\r\\n` when the texts do.",
      "Changes are moved to the latest place where they still fit, as git does, so a block of added lines ends before the line that follows it rather than in the middle of a repeated one.",
      "The time grows with the size of the texts and the number of differences. Two versions of a 50 000-line file with changes all over it compare in tens of milliseconds; two unrelated texts are fast with `histogram` and slow with `myers`.",
    ],
    edgeCases: [
      "`TypeError`: a text is not a string or bytes (`words()`, `chars()` and `merge()` take strings only), an option is not one of its values, only one of `oldName` and `newName` is given, or `labels` is not three strings without line breaks.",
      "`RangeError`: `context` is not a non-negative integer.",
      "`changesAsync()` rejects with the same errors instead of throwing.",
    ],
    examples: [
      {
        title: "Highlight what changed in a file",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

const changes = await TextDiff.changesAsync(saved, edited, { inline: "words" });
for (const change of changes) {
  markLines("removed", change.oldStart, change.oldEnd);
  markLines("added", change.newStart, change.newEnd);
  for (const part of change.inline ?? []) {
    markRange("removed", part.oldFrom, part.oldTo); // offsets into saved
    markRange("added", part.newFrom, part.newTo); // offsets into edited
  }
}`,
      },
      {
        title: "Compare two files without strings",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

// Bytes as a Uint8Array or a NativeBuffer, read as UTF-8 in the host.
const changes = TextDiff.changes(before, after, { ignoreWhitespace: "amount" });`,
      },
      {
        title: "The edit inside one line",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

TextDiff.words("let total = a + b;", "let sum = a + b;");
// [
//   { op: "equal", text: "let " },
//   { op: "delete", text: "total" },
//   { op: "insert", text: "sum" },
//   { op: "equal", text: " = a + b;" },
// ]`,
      },
      {
        title: "Merge two edits of one text",
        lang: "ts",
        code: `import { TextDiff } from "skathi:text";

const { text, conflicts } = TextDiff.merge(base, mine, theirs, { style: "zdiff3" });
if (conflicts.length) {
  for (const c of conflicts) console.log(\`conflict at line \${c.at + 1}\`);
} else {
  save(text);
}`,
      },
    ],
    related: ["patch", "text-search", "string-reader"],
    aliases: [
      "diff",
      "unified diff",
      "histogram",
      "Myers",
      "patience",
      "compare texts",
      "word diff",
      "inline diff",
      "three-way merge",
      "merge",
      "conflict",
      "diff3",
      "DiffHunk",
      "DiffOp",
      "DiffChange",
      "changes",
    ],
  },
  {
    slug: "patch",
    name: "Patch",
    kind: "class",
    summary:
      "A unified diff as git writes and reads it: parsed from git's output or made from two texts, applied, cut down to some hunks or lines, and reversed.",
    syntax: `${importLine("skathi:text", "Patch")}

Patch.parse(text: string): Patch
Patch.from(oldText: DiffSource, newText: DiffSource, options?: DiffOptions & {
  oldName?: string
  newName?: string
}): Patch

readonly patch.files: PatchFile[]
patch.apply(text: string, options?: { file?: number; fuzz?: number; reverse?: boolean }): {
  text: string
  rejected: number[]
}
patch.select(
  picks: { file: number; hunks?: number[]; lines?: [hunk: number, line: number][] }[],
  options?: { reverse?: boolean },
): Patch
patch.reverse(): Patch
patch.toString(): string

interface PatchFile {
  oldPath: string | null; newPath: string | null   // null for /dev/null
  status: "modified" | "added" | "deleted" | "renamed" | "copied"
  oldMode?: number; newMode?: number               // 0o100644, 0o100755, ...
  binary: boolean
  hunks: DiffHunk[]                                // with section and noNewline where the patch has them
}`,
    params: [
      {
        name: "text",
        type: "string",
        description:
          "For `parse()`: the output of `git diff`, `git show`, `git format-patch` or `diff -u`. Text around the diffs, such as a commit message, is skipped.",
      },
      {
        name: "options",
        type: "DiffOptions",
        description:
          "For `from()`: the options of [`TextDiff`](/std/text-diff/), and `oldName` and `newName` for the `---` and `+++` lines.",
      },
      { name: "options.file", type: "number", description: "For `apply()`: which file of the patch. 0 by default." },
      {
        name: "options.fuzz",
        type: "number",
        description:
          "For `apply()`: how many context lines a hunk may drop at each end to find its place. 0 by default, as `git apply`.",
      },
      {
        name: "options.reverse",
        type: "boolean",
        description: "For `apply()`: undo the patch instead.",
      },
      {
        name: "picks",
        type: "object[]",
        description:
          "For `select()`: per file, the hunks to take whole and single lines as `[hunk, line]`, indices into `files[file].hunks[hunk].lines`. A file given without either is taken whole.",
      },
      {
        name: "options.reverse",
        type: "boolean",
        description:
          "For `select()`: the patch that takes the picked lines back out of the new side, with the lines still picked by their indices in this patch.",
      },
    ],
    notes: [
      "`parse()` reads git's extended headers: new and deleted files, renames and copies with their similarity, mode changes, the `index` line, quoted paths with unusual characters, `Binary files … differ` and `GIT binary patch`, and `\\ No newline at end of file`. Paths come without git's `a/` and `b/`. `toString()` writes the same form back, so a patch read from git and cut down goes back to git.",
      "`select()` is how a git client stages one hunk or a few lines: an unpicked deleted line stays as context, an unpicked added line is left out, and the line numbers of the hunks after it are counted again. Pass the result's `toString()` to `git apply --cached` on its standard input.",
      "`apply()` applies one file's hunks to a text, in order. A hunk goes where its line numbers say when its lines are there; otherwise it is looked for further away in both directions, so a file that changed above it still takes it. A hunk whose context stops at the start or the end of the file stays there, as in git. A hunk that finds no place is skipped and its index is in `rejected`; the others are still applied.",
      "`select(picks, { reverse: true })` is how a git client unstages or discards a few lines: the result applies to the new side (the index for `git diff --cached`, the work tree for `git diff`) and leaves every other line as it is there. An unpicked added line stays as context and an unpicked deleted line is left out, as `git checkout -p` does. Pass its `toString()` to `git apply --cached` or `git apply`.",
      "`reverse()` swaps the old and new sides: paths, modes, added and deleted, the `index` hashes, and the lines of every hunk. Within a run of changes the deleted lines come first, as git writes them, so a line can have another index in the reversed patch than in this one. Pick lines from the patch the user sees and pass `reverse: true` to `select()` rather than picking from `reverse()`.",
      "`files` is made on first read and kept; the patch itself does not change, and `select()` and `reverse()` return new patches.",
      "Only text is applied: a binary file's part is kept in `toString()` but `apply()` refuses it.",
    ],
    edgeCases: [
      "`SyntaxError`: `parse()` meets a hunk whose header is malformed or whose lines do not match its counts; the message names the line.",
      "`TypeError`: `new Patch()`; a text is not a string; `apply()` on a binary file; a pick is not an object or a line is not `[hunk, line]`.",
      "`RangeError`: a file, hunk or line index that the patch does not have, or a negative or fractional `fuzz`.",
    ],
    examples: [
      {
        title: "Stage three lines of a file",
        lang: "ts",
        code: `import { Process } from "skathi:process";
import { Patch } from "skathi:text";

const { stdout } = await Process.exec("git", ["diff", "--", "src/app.ts"]);
const patch = Patch.parse(stdout);

// Lines 4 to 6 of the first hunk, as the user selected them.
const staged = patch.select([{ file: 0, lines: [[0, 4], [0, 5], [0, 6]] }]);

const git = Process.spawn("git", ["apply", "--cached", "-"]);
const input = git.stdin!.getWriter();
await input.write(new TextEncoder().encode(staged.toString()));
await input.close();
await git.exited;`,
      },
      {
        title: "Discard two lines from the work tree",
        lang: "ts",
        code: `import { Process } from "skathi:process";
import { Patch } from "skathi:text";

const { stdout } = await Process.exec("git", ["diff", "--", "src/app.ts"]);
const patch = Patch.parse(stdout);

// The same indices the view showed; the result applies to the work tree.
const discard = patch.select([{ file: 0, lines: [[0, 4], [0, 5]] }], { reverse: true });

const git = Process.spawn("git", ["apply", "-"]);
const input = git.stdin!.getWriter();
await input.write(new TextEncoder().encode(discard.toString()));
await input.close();
await git.exited;`,
      },
      {
        title: "Show a commit",
        lang: "ts",
        code: `import { Patch } from "skathi:text";

for (const file of Patch.parse(show).files) {
  console.log(file.status, file.newPath ?? file.oldPath, file.binary ? "(binary)" : \`\${file.hunks.length} hunks\`);
}`,
      },
      {
        title: "Apply a patch to a text, and undo it",
        lang: "ts",
        code: `import { Patch } from "skathi:text";

const patch = Patch.from(draft, edited, { oldName: "a/notes.md", newName: "b/notes.md" });
const { text, rejected } = patch.apply(otherCopyOfDraft);
if (rejected.length) console.warn(\`\${rejected.length} hunks did not fit\`);

const undone = patch.apply(edited, { reverse: true }).text; // draft again`,
      },
    ],
    related: ["text-diff"],
    aliases: [
      "patch",
      "git diff",
      "git apply",
      "unified diff",
      "parse diff",
      "apply patch",
      "stage hunk",
      "partial staging",
      "add -p",
      "unstage lines",
      "discard lines",
      "checkout -p",
      "reverse patch",
      "PatchFile",
    ],
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
