// `skathi:text`: building and reading strings, differences, many-word search and glob patterns.
// The host installs the classes; an index is in UTF-16 code units, as everywhere in JS.

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

export interface DiffOp {
  op: "equal" | "delete" | "insert";
  text: string;
}

/** Changed lines with the unchanged ones around them. Line numbers start at 1. */
export interface DiffHunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  /** One entry per line, without its `\n`. */
  lines: DiffOp[];
}

export interface DiffOptions {
  /** Unchanged lines kept around a change, 3 by default. */
  context?: number;
}

export interface UnifiedDiffOptions extends DiffOptions {
  /** With both names the text starts with the `---` and `+++` lines. */
  oldName?: string;
  newName?: string;
}

export interface TextDiffStatics {
  /** The shortest set of line changes that turns `oldText` into `newText`; `[]` when they are equal. */
  lines(oldText: string, newText: string, options?: DiffOptions): DiffHunk[];
  /** The same by characters, as runs: joining the `equal` and `insert` runs gives `newText`. */
  chars(oldText: string, newText: string): DiffOp[];
  /** The line changes as `diff -u` prints them; `""` when the texts are equal. */
  unified(oldText: string, newText: string, options?: UnifiedDiffOptions): string;
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
    TextSearch: TextSearchConstructor;
    Glob: GlobConstructor;
  };
};

export const StringBuilder: StringBuilderConstructor = __sk_std.text.StringBuilder;
export const StringWriter: StringWriterConstructor = __sk_std.text.StringWriter;
export const StringReader: StringReaderConstructor = __sk_std.text.StringReader;
export const TextDiff: TextDiffStatics = __sk_std.text.TextDiff;
export const TextSearch: TextSearchConstructor = __sk_std.text.TextSearch;
export const Glob: GlobConstructor = __sk_std.text.Glob;
