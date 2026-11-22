// `skathi:text`: building and reading strings, differences and patches, many-word search and glob
// patterns. The host installs the classes; an index is in UTF-16 code units, as everywhere in JS.

import type { NativeBuffer } from "./memory";

/** Text put together from parts in one buffer. */
export interface StringBuilder {
  /** In UTF-16 code units. */
  readonly length: number;
  /** Appends `String(value)`. */
  append(value: unknown): this;
  /** Appends the value, when there is one, and `"\n"`. */
  appendLine(value?: unknown): this;
  /** Appends every part in one call, with `separator` between them. */
  appendAll(parts: readonly unknown[], separator?: string): this;
  insert(index: number, text: string): this;
  clear(): void;
  toString(): string;
}

export interface StringBuilderConstructor {
  readonly prototype: StringBuilder;
  new (initial?: string): StringBuilder;
}

/** A sink for text, for code that writes and does not read back until the end. */
export interface StringWriter {
  readonly length: number;
  write(value: unknown): void;
  writeLine(value?: unknown): void;
  writeAll(parts: readonly unknown[], separator?: string): void;
  toString(): string;
}

export interface StringWriterConstructor {
  readonly prototype: StringWriter;
  new (): StringWriter;
}

/** A position in a string that moves forward by characters or by lines. */
export interface StringReader {
  /** Where the next read starts, in UTF-16 code units. */
  readonly position: number;
  readonly length: number;
  /** The next character, a surrogate pair as one; `undefined` at the end. */
  read(): string | undefined;
  /** What `read()` would return, without moving. */
  peek(): string | undefined;
  /** The next line without its `\n`, `\r\n` or `\r`; `null` at the end. */
  readLine(): string | null;
  /** Every line left, or `max` of them, in one call. */
  readLines(max?: number): string[];
  readToEnd(): string;
}

export interface StringReaderConstructor {
  readonly prototype: StringReader;
  new (text: string): StringReader;
}

/** A text to compare: a string, or bytes read as UTF-8. */
export type DiffSource = string | Uint8Array | NativeBuffer;

export interface DiffOp {
  op: "equal" | "delete" | "insert";
  text: string;
  /** In a parsed patch: `\ No newline at end of file` follows the line. */
  noNewline?: true;
}

/** Changed lines with the unchanged ones around them. Line numbers start at 1. */
export interface DiffHunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  /** In a parsed patch: what follows the closing `@@`, usually the enclosing function. */
  section?: string;
  /** One entry per line, without its `\n`. */
  lines: DiffOp[];
}

export interface DiffOptions {
  /** `"histogram"` by default; `chars()` uses `"myers"`. */
  algorithm?: "histogram" | "myers" | "patience";
  ignoreWhitespace?: "none" | "trailing" | "amount" | "all";
  ignoreCase?: boolean;
  /** `\r\n` and `\n` are the same line ending. */
  ignoreLineEndings?: boolean;
  /** Unchanged lines kept around a change, 3 by default. */
  context?: number;
  /** For `changes()`: the parts that differ inside replaced lines, by words or by characters. */
  inline?: "none" | "words" | "chars";
}

export interface UnifiedDiffOptions extends DiffOptions {
  /** With both names the text starts with the `---` and `+++` lines. */
  oldName?: string;
  newName?: string;
}

/** Offsets in UTF-16 code units from the start of each text. */
export interface InlineChange {
  oldFrom: number;
  oldTo: number;
  newFrom: number;
  newTo: number;
}

/** A change as two ranges of lines, from 0, the end not included; no text is copied. */
export interface DiffChange {
  oldStart: number;
  oldEnd: number;
  newStart: number;
  newEnd: number;
  /** When `inline` is asked for; empty when one side has no lines or the change is very long. */
  inline?: InlineChange[];
}

export interface MergeOptions {
  /** `"merge"` by default; `"diff3"` and `"zdiff3"` also show the base lines. */
  style?: "merge" | "diff3" | "zdiff3";
  /** The names after the markers, in argument order: base, ours, theirs. */
  labels?: [string, string, string];
}

/** Line ranges, from 0, the end not included; `at` is the line of `<<<<<<<` in `text`. */
export interface MergeConflict {
  base: [number, number];
  ours: [number, number];
  theirs: [number, number];
  at: number;
}

export interface MergeResult {
  /** With conflict markers where there are conflicts. */
  text: string;
  conflicts: MergeConflict[];
}

export interface TextDiffStatics {
  /** The line changes that turn `oldText` into `newText` as ranges; `[]` when they are equal. */
  changes(oldText: DiffSource, newText: DiffSource, options?: DiffOptions): DiffChange[];
  /** The same on a worker thread. */
  changesAsync(oldText: DiffSource, newText: DiffSource, options?: DiffOptions): Promise<DiffChange[]>;
  /** The line changes with their text, in hunks with context. */
  lines(oldText: DiffSource, newText: DiffSource, options?: DiffOptions): DiffHunk[];
  /** By words, as runs: joining the `equal` and `insert` runs gives `newText`. */
  words(oldText: string, newText: string, options?: DiffOptions): DiffOp[];
  /** By characters, as runs; a surrogate pair is never split. */
  chars(oldText: string, newText: string, options?: DiffOptions): DiffOp[];
  /** The line changes as `diff -u` prints them; `""` when the texts are equal. */
  unified(oldText: DiffSource, newText: DiffSource, options?: UnifiedDiffOptions): string;
  /** Both sides' changes to `base` in one text, with conflict markers where they overlap. */
  merge(base: string, ours: string, theirs: string, options?: MergeOptions): MergeResult;
}

/** One file's part of a patch. Paths have no `a/` or `b/`; `null` stands for `/dev/null`. */
export interface PatchFile {
  oldPath: string | null;
  newPath: string | null;
  status: "modified" | "added" | "deleted" | "renamed" | "copied";
  /** As numbers: `0o100644`, `0o100755`. */
  oldMode?: number;
  newMode?: number;
  binary: boolean;
  hunks: DiffHunk[];
}

export interface PatchApplyOptions {
  /** Which file of the patch, 0 by default. */
  file?: number;
  /** Context lines a hunk may drop at each end to find its place, 0 by default. */
  fuzz?: number;
  reverse?: boolean;
}

export interface PatchResult {
  text: string;
  /** The hunks that found no place, by index; the others are applied. */
  rejected: number[];
}

export interface PatchPick {
  file: number;
  /** Whole hunks, by index. */
  hunks?: number[];
  /** Single lines as `[hunk, line]`, indices into `files[file].hunks[hunk].lines`. */
  lines?: [number, number][];
}

/** A unified diff, as git writes and reads it. */
export interface Patch {
  readonly files: PatchFile[];
  apply(text: string, options?: PatchApplyOptions): PatchResult;
  /** A patch of the picked files, hunks and lines; a file picked without either is taken whole. */
  select(picks: readonly PatchPick[]): Patch;
  reverse(): Patch;
  toString(): string;
}

export interface PatchConstructor {
  readonly prototype: Patch;
  /** A unified diff with git's extended headers, as `git diff` and `git show` print it. */
  parse(text: string): Patch;
  from(oldText: DiffSource, newText: DiffSource, options?: UnifiedDiffOptions): Patch;
}

export interface TextMatch {
  /** In UTF-16 code units, as `String.prototype.indexOf` answers. */
  index: number;
  length: number;
  /** The position of the pattern in the list the search was made from. */
  pattern: number;
}

export interface TextSearchOptions {
  ignoreCase?: boolean;
  /** A match must not touch a letter, a digit or `_` on either side. */
  wholeWord?: boolean;
}

/** Many fixed strings looked for in one pass over the text. */
export interface TextSearch {
  /** The matches from left to right, none overlapping another; of those that start together, the longest. */
  findAll(text: string): TextMatch[];
  test(text: string): boolean;
}

export interface TextSearchConstructor {
  readonly prototype: TextSearch;
  new (patterns: readonly string[], options?: TextSearchOptions): TextSearch;
}

export interface GlobOptions {
  /** `true` by default. */
  caseSensitive?: boolean;
}

/** A path pattern: `*`, `**`, `?`, `[a-z]`, `{a,b}`, `\` to escape, a leading `!` to negate. */
export interface Glob {
  match(path: string): boolean;
  /** The paths that match, in one call. */
  filter(paths: readonly string[]): string[];
}

export interface GlobConstructor {
  readonly prototype: Glob;
  new (pattern: string, options?: GlobOptions): Glob;
}

declare const __sk_std: {
  text: {
    StringBuilder: StringBuilderConstructor;
    StringWriter: StringWriterConstructor;
    StringReader: StringReaderConstructor;
    TextDiff: TextDiffStatics;
    Patch: PatchConstructor;
    TextSearch: TextSearchConstructor;
    Glob: GlobConstructor;
  };
};

export const StringBuilder: StringBuilderConstructor = __sk_std.text.StringBuilder;
export const StringWriter: StringWriterConstructor = __sk_std.text.StringWriter;
export const StringReader: StringReaderConstructor = __sk_std.text.StringReader;
export const TextDiff: TextDiffStatics = __sk_std.text.TextDiff;
export const Patch: PatchConstructor = __sk_std.text.Patch;
export const TextSearch: TextSearchConstructor = __sk_std.text.TextSearch;
export const Glob: GlobConstructor = __sk_std.text.Glob;
