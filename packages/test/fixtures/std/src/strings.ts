import { NativeBuffer } from "skathi:memory";
import {
  FuzzyMatcher,
  Glob,
  Patch,
  StringBuilder,
  StringReader,
  StringWriter,
  TextDiff,
  TextSearch,
} from "skathi:text";

const attempt = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

export function text() {
  const builder = new StringBuilder("a");
  builder
    .append(1)
    .append(null)
    .appendLine()
    .appendAll(["x", 2, { toString: () => "obj" }], ", ")
    .insert(1, "😀");
  const writer = new StringWriter();
  writer.write("total: ");
  writer.writeLine(3);
  writer.writeAll(["a", "b"], "|");

  const reader = new StringReader("😀b\r\nsecond\rthird\n\nlast");
  const first = [reader.peek(), reader.read(), reader.position, reader.read()];
  const restOfLine = reader.readLine();
  const two = reader.readLines(2);
  const tail = [reader.readLine(), reader.readToEnd(), reader.readLine(), reader.read() ?? "end"];

  const before = "one\ntwo\nthree\n";
  const after = "one\n2\nthree\nfour\n";
  const chars = TextDiff.chars("kitten 😀", "sitting 😀");

  const search = new TextSearch(["он", "она", "Кот"], { ignoreCase: true });
  const words = new TextSearch(["cat"], { wholeWord: true });
  const sentence = "😀 Она и кот";
  const ranked = FuzzyMatcher.rank("дс", ["😀ДверьСада", "дс", "сад", "ДС"]);

  const sources = new Glob("src/**/*.{ts,vue}");
  return {
    builder: [builder.toString(), builder.length],
    cleared: (builder.clear(), builder.length),
    writer: writer.toString(),
    reader: [first, restOfLine, two, tail],
    hunks: TextDiff.lines(before, after, { context: 1 }),
    unified: TextDiff.unified(before, after, { oldName: "a.txt", newName: "b.txt" }),
    same: [TextDiff.lines(before, before).length, TextDiff.unified(before, before)],
    chars,
    rebuilt: chars
      .filter((part) => part.op !== "delete")
      .map((part) => part.text)
      .join(""),
    found: search.findAll(sentence).map((m) => [sentence.slice(m.index, m.index + m.length), m.pattern]),
    words: [words.test("concat"), words.test("a cat."), new TextSearch([]).test("anything")],
    fuzzy: {
      matches: ranked.map(({ index, ranges }) => [index, ranges.map((r) => [r.index, r.length])]),
      scores: ranked.map((m) => m.score > ranked[ranked.length - 1]!.score),
      limited: FuzzyMatcher.rank("😀", ["a😀", "😀"], { limit: 1 }),
      overlap: FuzzyMatcher.rank("test", ["t-e-s-test"])[0]?.ranges,
      empty: FuzzyMatcher.rank("", ["b", "a"], { limit: 1 }),
      invalid: attempt(() => FuzzyMatcher.rank("a", ["a"], { limit: -1 })).split(":")[0],
    },
    glob: [
      sources.match("src/a/b/App.vue"),
      sources.match("src/main.ts"),
      sources.match("lib/main.ts"),
      new Glob("*.MD", { caseSensitive: false }).match("readme.md"),
      new Glob("!*.md").filter(["a.md", "b.ts", "c.md"]),
    ],
    errors: [
      attempt(() => new TextSearch("cat" as unknown as string[])),
      attempt(() => new TextSearch([""])),
      attempt(() => new StringBuilder().insert(5, "x")),
      attempt(() => TextDiff.lines("a", 1 as unknown as string)),
      attempt(() => (StringBuilder.prototype.append as (v: unknown) => unknown).call({}, "x")),
    ],
  };
}

const GIT_DIFF = `diff --git a/src/app.ts b/src/app.ts
index 83db48f..bf269f4 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,4 +1,4 @@ export function main() {
 one
-two
+2
 three
 four
@@ -8,3 +8,4 @@
 eight
 nine
 ten
+eleven
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
diff --git a/old.txt b/new.txt
similarity index 100%
rename from old.txt
rename to new.txt
`;

/** Differences, merges and patches, synchronous. */
export function diffs() {
  const file = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
  const patch = Patch.parse(GIT_DIFF);
  const staged = patch.select([{ file: 0, lines: [[1, 3]] }]);
  const fromText = Patch.from("a\nb\n", "a\nB\n", { oldName: "a/x", newName: "b/x" });
  const bytes = new Uint8Array([0xf0, 0x9f, 0x98, 0x80, 0x20, 0x62, 0x0a, 0x63, 0x0a]); // "😀 b\nc\n"
  const buffer = NativeBuffer.alloc(bytes.length);
  new Uint8Array(buffer.buffer).set(bytes);
  const inline = TextDiff.changes("😀 a\nc\n", buffer, { inline: "words" });
  buffer.free();
  const merged = TextDiff.merge("a\nb\nc\n", "A\nb\nc\n", "a\nb\nC\n");
  const conflict = TextDiff.merge("a\n", "ours\n", "theirs\n", { style: "diff3" });
  return {
    changes: TextDiff.changes("a\nb\nc\n", "a\nx\nc\nd\n"),
    inline,
    ignored: [
      TextDiff.changes("a b\r\n", "a  b\n", { ignoreWhitespace: "amount", ignoreLineEndings: true }).length,
      TextDiff.changes("A\n", "a\n", { ignoreCase: true }).length,
    ],
    words: TextDiff.words("let total = a + b;", "let sum = a + b;"),
    chars: TextDiff.chars("Abc", "abc", { ignoreCase: true }).length,
    unified: TextDiff.unified("a\nb\n", "a\nB\n", { context: 0 }),
    merged,
    conflict,
    files: patch.files.map((f) => [f.oldPath, f.newPath, f.status, f.oldMode, f.newMode, f.hunks.length]),
    section: patch.files[0]?.hunks[0]?.section,
    applied: patch.apply(file),
    moved: patch.apply(file.replace("five\n", "five\nfive and a half\n")).text.endsWith("ten\neleven\n"),
    back: patch.reverse().apply(patch.apply(file).text).text === file,
    staged: staged.toString(),
    stagedApplied: staged.apply(file).text,
    roundTrip: Patch.parse(patch.toString()).toString() === patch.toString(),
    fromText: [fromText.toString(), fromText.apply("a\nb\n").text],
    rejected: Patch.from("1\n2\n3\n", "1\nX\n3\n").apply("1\nY\n3\n"),
    errors: [
      attempt(() => Patch.parse("@@ -1,2 +1,2 @@\n a\n")),
      attempt(() => new (Patch as unknown as new () => Patch)()),
      attempt(() => patch.apply(file, { file: 9 })),
      attempt(() => patch.select([{ file: 0, hunks: [5] }])),
      attempt(() => TextDiff.changes("a", 5 as unknown as string)),
      attempt(() => TextDiff.lines("a", "b", { algorithm: "fast" as "myers" })),
    ],
  };
}

/** A 50 000-line file with changes all over it, compared on a worker. */
export async function diffsAsync() {
  const lines = Array.from({ length: 50_000 }, (_, i) => `  const value${i} = compute(${i % 97}, ${i % 13});`);
  const old = lines.join("\n") + "\n";
  for (let i = 250; i < lines.length; i += 500) lines[i] = `  const value${i} = changed(${i});`;
  const changed = lines.join("\n") + "\n";
  const changes = await TextDiff.changesAsync(old, changed, { inline: "words" });
  const first = changes[0];
  const part = first?.inline?.[0];
  return {
    count: changes.length,
    first: first && [first.oldStart, first.oldEnd, first.newStart, first.newEnd],
    part: part && [old.slice(part.oldFrom, part.oldTo), changed.slice(part.newFrom, part.newTo)],
    sameAsSync: JSON.stringify(TextDiff.changes(old, changed, { inline: "words" })) === JSON.stringify(changes),
    rejected: await TextDiff.changesAsync(old, 1 as unknown as string).then(
      () => "ok",
      (e: Error) => e.name,
    ),
  };
}
