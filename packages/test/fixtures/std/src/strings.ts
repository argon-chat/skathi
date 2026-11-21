import { Glob, StringBuilder, StringReader, StringWriter, TextDiff, TextSearch } from "skathi:text";

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
