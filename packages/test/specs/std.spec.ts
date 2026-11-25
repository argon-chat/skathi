import { afterAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { arch, homedir, hostname, platform } from "node:os";
import { killTree, launch, type SkApp } from "../src/index";

const T = 60_000;
const apps: SkApp[] = [];
const started = new Map<number, RegExp>();
afterAll(async () => {
  for (const [pid, image] of started) killTree(pid, image);
  for (const a of apps) await a.close();
});

async function start() {
  const app = await launch({ app: "packages/test/fixtures/std/src/main.ts", window: [420, 300], label: "std" });
  apps.push(app);
  await app.locator("#ready").waitFor();
  return app;
}

async function ended(app: SkApp): Promise<number | null> {
  const deadline = Date.now() + 10_000;
  while (app.child.exitCode === null && Date.now() < deadline) await Bun.sleep(20);
  return app.child.exitCode;
}

describe("skathi:* modules", () => {
  test(
    "skathi:os, skathi:runtime and skathi:app answer for this machine and this app",
    async () => {
      const app = await start();
      const p = JSON.parse(String(await app.eval("__SK_TEST_STD__.probe()")));
      expect(p.version).toBe("4.5.6");
      expect(p.sameShell).toBe(true);
      expect(p.os.platform).toBe(platform());
      expect(p.os.arch).toBe(arch());
      expect(["x64", "arm64"]).toContain(p.os.machineArch);
      expect(p.os.release).toMatch(/^\d+\.\d+/);
      expect(p.os.hostname.toLowerCase()).toBe(hostname().toLowerCase());
      expect(p.os.homedir).toBe(homedir());
      expect(p.os.tmpdir).not.toMatch(/[\\/]$/);
      expect(p.os.isOS).toBe(true);
      expect(p.os.frozenDistro).toBe(true);
      if (platform() !== "linux") expect(p.os.distro).toBeNull();
      expect(p.runtime.version).toMatch(/^\d+\.\d+\.\d+/);
      expect(Array.isArray(p.runtime.features)).toBe(true);
      expect(p.runtime.isAot).toBe(false);
      expect(p.runtime.isSingleFile).toBe(false);
      expect(p.badExitCode).toStartWith("TypeError");
      expect(p.badArgs).toStartWith("TypeError");
      expect(app.child.exitCode).toBeNull();
    },
    T,
  );

  test(
    "skathi:toml parses into JS values and writes them back",
    async () => {
      const app = await start();
      const t = JSON.parse(String(await app.eval("__SK_TEST_STD__.toml()")));
      expect(t.keys).toEqual(["title", "big", "ratio", "when", "day", "tags", "window", "servers"]);
      expect([t.title, t.big, t.bigType, t.ratio]).toEqual(["Skathi", "9007199254740993", "bigint", 0.5]);
      expect(t.when).toEqual(["Date", "1979-05-27T07:32:00.000Z"]);
      expect(t.day).toEqual(["Temporal.PlainDate", "1979-05-27"]);
      expect(t.window).toEqual({ width: 800, resizable: true });
      expect(t.servers).toEqual([{ name: "one" }]);
      expect(t.round).toBe(true);
      expect(t.written).toBe(
        'name = "x"\nn = 4611686018427387904\nat = 1970-01-01T00:00:00.0Z\nlist = [1, 2]\n\n[t]\nok = true\n',
      );
      expect(t.syntax).toStartWith("SyntaxError: TOML.parse: line 2, column 14:");
      expect(t.nil).toBe("TypeError: TOML.stringify: window.title is null, which TOML cannot represent");
      expect(t.hole).toStartWith("TypeError: TOML.stringify: list[1] is undefined");
      expect(t.notTable).toStartWith("TypeError");
    },
    T,
  );

  test(
    "skathi:ffi calls C functions of the libraries the config lists, and only those",
    async () => {
      const app = await start();
      const f = JSON.parse(String(await app.eval("__SK_TEST_STD__.ffi()")));
      expect([f.strlen, f.strlenType, f.abs, f.ldexp]).toEqual(["6", "bigint", [5, 5, 7], 1024]);
      expect([f.copied, f.stable]).toEqual([[1, 2, 3, 4], true]);
      expect([f.slash, f.slashAt, f.offset, f.bytes]).toEqual(["/ffi", 6, "ff", [115, 107, 97]]);
      expect([f.sameStart, f.nullFound, f.byPointer]).toEqual([true, true, "10"]);
      expect([f.line, f.missing, f.names]).toEqual(["-7 2.50 ok", null, ["strlen", "usize", "crt"]]);
      const e = f.errors;
      expect(e.notListed).toStartWith('NotAllowedError: dlopen: the library "gdi32" is not in "permissions.ffi"');
      expect(e.noSymbol).toStartWith("NotFoundError: dlopen: crt.sk_no_such_function");
      expect([e.badType, e.count, e.text, e.fraction, e.bufferAsPointer, e.nul].map((m) => m.split(":")[0])).toEqual(
        Array(6).fill("TypeError"),
      );
      expect(e.range).toBe("RangeError: crt.abs: argument 1 (i32): 2147483648 does not fit i32");
      expect(e.address).toStartWith("NotAllowedError: NativePointer.fromAddress");
      expect([e.nullRead, e.construct].map((m) => m.split(":")[0])).toEqual(["TypeError", "TypeError"]);
      expect(e.closed).toBe("InvalidStateError: crt.abs: the library is closed");
      if (platform() === "win32") {
        const w = f.windows;
        expect(w.lastError).toEqual({ errno: 1234 });
        expect([w.pid, w.wide, w.module]).toEqual([true, 10, false]);
        expect(w.systemRoot).toBe((process.env.SystemRoot ?? "").toLowerCase());
        expect([w.unsigned, w.top]).toEqual([[2147483647, 2147483648], 4294967295]);
        expect([w.big, w.bigType]).toEqual(["-9007199254740993", "bigint"]);
      }
    },
    T,
  );

  test(
    "skathi:ffi passes and returns structs by value",
    async () => {
      const app = await start();
      await app.eval("__SK_TEST_STD__.startStructs(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.structsResult)"));
      }
      const s = JSON.parse(raw);
      expect(s.failed).toBeUndefined();
      const r = s.result;
      expect([r.div, r.later]).toEqual([
        { quot: -3, rem: -2 },
        { quot: 3, rem: 2 },
      ]);
      expect(r.far).toEqual(["-1048576", "-3"]);
      expect(r.layout).toEqual([48, 8, { tag: 0, inner: 8, list: 24, next: 40 }, 16]);
      expect(r.packed).toEqual([48, 7, 255, 1, 254]);
      expect(r.read).toEqual([7, { a: 255, b: 0.5 }, [1, -2, 3], true]);
      if (platform() === "win32") expect(r.windows).toEqual([1, 0, 1]);
      else expect(r.dotted).toBe("127.0.0.1");
      const e = r.errors;
      expect(e.pointer).toStartWith("NotAllowedError: unpack: a pointer read from bytes");
      expect(e.short).toStartWith("RangeError: unpack: the struct is 48 bytes");
      expect(e.field).toBe("TypeError: pack: the value: field list must be an array of 3 elements");
      expect(e.value).toBe("RangeError: pack: the value: field inner: field a: 256 does not fit u8");
      expect([e.empty, e.type, e.variadic, e.callback].map((m) => m.split(":")[0])).toEqual(Array(4).fill("TypeError"));
      if (platform() !== "win32") {
        expect(e.bytes).toBe("TypeError: crt.inet_ntoa: argument 1 (struct) is 8 bytes, and the struct is 4");
      }
    },
    T,
  );

  test(
    "a NativeCallback is called by the library, on its thread or from another",
    async () => {
      const app = await start();
      const c = JSON.parse(String(await app.eval("__SK_TEST_STD__.callbacks()")));
      expect([c.sorted, c.called]).toEqual([[40, 12, 7, 5, 3, 0, -1, -8], true]);
      expect(c.thrown).toBe("RangeError: not comparable");
      expect(c.wrongAnswer).toStartWith("TypeError: NativeCallback (anonymous): the value it returns");
      expect([c.afterThrow, c.open, c.left]).toEqual(["ok", 3, 0]);
      expect(c.closed).toEqual([true, "InvalidStateError: NativeCallback.pointer: the callback is closed"]);
      expect([c.notFunction, c.buffer, c.text].map((m) => m.split(":")[0])).toEqual(Array(3).fill("TypeError"));

      await app.eval("__SK_TEST_STD__.startThreads(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.threadsResult)"));
      }
      const t = JSON.parse(raw);
      expect(t.failed).toBeUndefined();
      if (platform() === "win32") {
        expect(t.result).toEqual({ accepted: [1, 1, 1], inline: 0, onJsThread: true, context: true, waited: 0 });
      }
      expect(app.child.exitCode).toBeNull();
    },
    T,
  );

  test(
    "a nonblocking function runs on a worker and answers with a promise",
    async () => {
      const app = await start();
      await app.eval("__SK_TEST_STD__.startWorkers(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.workersResult)"));
      }
      const w = JSON.parse(raw);
      expect(w.failed).toBeUndefined();
      const r = w.result;
      expect([r.promise, r.length, r.copied, r.many]).toEqual([true, "6", false, 780]);
      expect([r.untouched, r.written]).toEqual([
        [0, 0, 0, 0],
        [9, 8, 7, 6],
      ]);
      expect(r.badType).toStartWith("TypeError: crt.abs: argument 1 (i32)");
      expect(r.resizable).toStartWith("TypeError: crt.memcpy: argument 1 (buffer) is resizable");
      if (platform() === "win32") {
        expect(r.windows).toEqual({ order: ["timer", "call"], lastError: { errno: 77 }, slept: true });
      }
    },
    T,
  );

  test(
    "skathi:binary writes a record and reads the same values back",
    async () => {
      const app = await start();
      const b = JSON.parse(String(await app.eval("__SK_TEST_STD__.binary()")));
      expect([b.written, b.emptied, b.remaining]).toEqual([34, 0, 0]);
      expect(b.head).toEqual({ magic: 0xcafebabe, size: 34, id: "-2", idType: "bigint" });
      expect([b.floats, b.word, b.vars, b.text]).toEqual([[1.5, -2], 0x0102, [624485, 127], "héllo"]);
      expect(b.past).toBe("RangeError: BinaryReader.bytes: 1 bytes wanted, 0 remain");
      expect([b.window, b.shared]).toEqual([[7, 6], true]);
      expect([b.bits, b.firstBits, b.nextBits]).toEqual([[205, 1], 5, [1, 2, 3]]);
      expect(b.bitsLeft).toStartWith("RangeError");
      expect([b.swapped, b.native]).toEqual([0x3412, "little"]);
      expect([b.varInts, b.varSize]).toEqual([[0, 300, -1, 2 ** 40], 2]);
      expect(b.negative).toStartWith("RangeError");
      expect([b.crc32, b.crc32Chained, b.crc32Stream]).toEqual([0xcbf43926, 0xcbf43926, 0xcbf43926]);
      expect([b.crc64, b.crc64Nvme]).toEqual(["995dc9bbdf1939fa", "ae8b14860a799888"]);
      expect(b.badType).toStartWith('TypeError: BinaryReader.readStruct: field "a" has the type "u24"');
      expect(b.globals).toEqual([]);
    },
    T,
  );

  test(
    "skathi:encoding turns bytes into text and names their type",
    async () => {
      const app = await start();
      const e = JSON.parse(String(await app.eval("__SK_TEST_STD__.encoding()")));
      expect([e.hex, e.unhex]).toEqual(["00ff10", [0, 255, 16]]);
      expect(e.oddHex).toStartWith("SyntaxError: Hex.decode:");
      expect(e.format).toEqual(["0x00ff", "-0x1", "0x10000000000000000"]);
      expect(e.dump).toStartWith("00000000: 4869 21");
      expect(e.dump).toEndWith("  Hi!\n");
      expect([e.base32, e.base32Back, e.crockford]).toEqual(["MZXW6YTBOI======", "foobar", "ZW010"]);
      expect([e.base64, e.base64Url, e.base64Back]).toEqual(["+/8=", "-_8", [251, 255]]);
      expect(e.base64Strict).toStartWith("SyntaxError: Base64.decode:");
      expect(e.mime).toEqual(["image/png", ""]);
      expect(e.extensions).toContain("jpg");
      expect(e.sniff).toEqual(["image/png", null]);
      expect(e.matches).toEqual([true, false]);
      expect([e.cyrillic, e.decoded]).toEqual([[0xc4, 0xe0, 0x88, 0x3f], "Да€?"]);
      expect(e.supported).toEqual([true, false]);
      expect(e.detect).toEqual(["utf-16le", null]);
      expect(e.unknown).toStartWith("RangeError: Encoding.decode:");
    },
    T,
  );

  test(
    "skathi:time measures, limits, retries and fires on the event loop",
    async () => {
      const app = await start();
      await app.eval("__SK_TEST_STD__.startTime(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.timeResult)"));
      }
      const t = JSON.parse(raw);
      expect(t.failed).toBeUndefined();
      expect(t.span).toEqual([5_430_000, 5430, "1h30m30s", 5_430_000, '{"span":5430000}']);
      expect(t.localized).toContain("30 min");
      expect(t.duration).toEqual(["PT1H30M30S", "Duration"]);
      expect(t.iso).toBe(86_400_500);
      expect(t.badSpan).toStartWith('SyntaxError: TimeSpan.parse: "90" is not a duration');
      expect([t.ordered, t.stopped, t.held, t.running]).toEqual([true, true, true, false]);
      expect([t.delays, t.jittered, t.attempts]).toEqual([[100, 200, 350], true, 3]);
      expect(t.badBackoff).toStartWith("RangeError");
      expect([t.before, t.sameSignal]).toEqual([[false, true, false], true]);
      expect(t.after).toEqual([true, 0, true, "TimeoutError"]);
      expect(t.passed).toBe("TimeoutError: The deadline has passed");
      expect([t.taken, t.waited, t.queued, t.refused]).toEqual([[true, true, false], true, "resolved", "AbortError"]);
      expect([t.retried, t.calls, t.exhausted, t.gaveUp, t.picky]).toEqual(["done", [1, 2, 3], "TypeError", 2, 1]);
      expect(t.stillPaused).toEqual([0, true, false]);
      expect(t.fired).toEqual([1, false]);
      expect(t.repeated).toBe(true);
      expect(t.globals).toEqual([]);
    },
    T,
  );

  test(
    "skathi:gpu lays a uniform struct out as the shader compiler does, times a pass and reads back",
    async () => {
      const app = await start();
      await app.eval("__SK_TEST_STD__.startGpu(); 1");
      let raw = "null";
      for (const until = Date.now() + 20_000; raw === "null" && Date.now() < until; await Bun.sleep(50))
        raw = String(await app.eval("String(__SK_TEST_STD__.gpuResult)"));
      const got = JSON.parse(raw);
      expect(got.failed).toBeUndefined();
      expect(got.messages).toEqual([]);
      expect(got.read).toEqual(got.expected);
      expect(got.pixels).toEqual(Array.from({ length: 24 }, (_, i) => i + 1));
      expect(got.row).toBe(12);
      expect(got.vertex).toEqual([255, 128, 0, 255]);
      if (got.timed !== null) expect(got.timed).toEqual(["copy"]);
    },
    T,
  );

  test(
    "skathi:units converts and formats sizes, frequencies and angles",
    async () => {
      const app = await start();
      const u = JSON.parse(String(await app.eval("__SK_TEST_STD__.units()")));
      expect([u.bytes, u.to]).toEqual([1_610_612_736, [1536, 2.5]]);
      expect(u.format.slice(0, 2)).toEqual(["1.5 GB", "1.5 GiB"]);
      expect(u.format[2]).toBe("999 B");
      expect(u.format[3]).toBe("1,5 MB");
      expect(u.larger).toBe(true);
      expect(u.badSize).toStartWith('SyntaxError: DataSize.parse: "3 parsecs" is not a size');
      expect(u.frequency).toEqual([20, 0.5]);
      expect(u.frequencyText).toEqual(["44.1 kHz", "1 kHz"]);
      expect(u.noPeriod).toStartWith("RangeError");
      expect(u.angle).toEqual([180, true, 270]);
      expect(u.angleText).toBe("90°");
    },
    T,
  );

  test(
    "skathi:text builds, reads, compares and searches strings",
    async () => {
      const app = await start();
      const t = JSON.parse(String(await app.eval("__SK_TEST_STD__.text()")));
      expect(t.builder).toEqual(["a😀1null\nx, 2, obj", 18]);
      expect(t.cleared).toBe(0);
      expect(t.writer).toBe("total: 3\na|b");
      expect(t.reader).toEqual([["😀", "😀", 2, "b"], "", ["second", "third"], ["", "last", null, "end"]]);
      expect(t.hunks).toEqual([
        {
          oldStart: 1,
          oldLines: 3,
          newStart: 1,
          newLines: 4,
          lines: [
            { op: "equal", text: "one" },
            { op: "delete", text: "two" },
            { op: "insert", text: "2" },
            { op: "equal", text: "three" },
            { op: "insert", text: "four" },
          ],
        },
      ]);
      expect(t.unified).toBe("--- a.txt\n+++ b.txt\n@@ -1,3 +1,4 @@\n one\n-two\n+2\n three\n+four\n");
      expect(t.same).toEqual([0, ""]);
      expect(t.rebuilt).toBe("sitting 😀");
      expect(t.chars.at(-1)).toEqual({ op: "equal", text: " 😀" });
      expect(t.found).toEqual([
        ["Она", 1],
        ["кот", 2],
      ]);
      expect(t.words).toEqual([false, true, false]);
      expect(t.glob).toEqual([true, true, false, true, ["b.ts"]]);
      expect(t.errors.map((e: string) => e.split(":")[0])).toEqual([
        "TypeError",
        "TypeError",
        "RangeError",
        "TypeError",
        "TypeError",
      ]);
    },
    T,
  );

  test(
    "skathi:text compares by lines and words, merges three ways, and parses, applies and cuts patches",
    async () => {
      const app = await start();
      const d = JSON.parse(String(await app.eval("__SK_TEST_STD__.diffs()")));
      expect(d.changes).toEqual([
        { oldStart: 1, oldEnd: 2, newStart: 1, newEnd: 2 },
        { oldStart: 3, oldEnd: 3, newStart: 3, newEnd: 4 },
      ]);
      expect(d.inline).toEqual([
        { oldStart: 0, oldEnd: 1, newStart: 0, newEnd: 1, inline: [{ oldFrom: 3, oldTo: 4, newFrom: 3, newTo: 4 }] },
      ]);
      expect(d.ignored).toEqual([0, 0]);
      expect(d.words).toEqual([
        { op: "equal", text: "let " },
        { op: "delete", text: "total" },
        { op: "insert", text: "sum" },
        { op: "equal", text: " = a + b;" },
      ]);
      expect(d.chars).toBe(1);
      expect(d.unified).toBe("@@ -2 +2 @@\n-b\n+B\n");
      expect(d.merged).toEqual({ text: "A\nb\nC\n", conflicts: [] });
      expect(d.conflict).toEqual({
        text: "<<<<<<< ours\nours\n||||||| base\na\n=======\ntheirs\n>>>>>>> theirs\n",
        conflicts: [{ base: [0, 1], ours: [0, 1], theirs: [0, 1], at: 0 }],
      });
      expect(d.files).toEqual([
        ["src/app.ts", "src/app.ts", "modified", 0o100644, 0o100644, 2],
        ["run.sh", "run.sh", "modified", 0o100644, 0o100755, 0],
        ["old.txt", "new.txt", "renamed", null, null, 0],
      ]);
      expect(d.section).toBe("export function main() {");
      expect(d.applied).toEqual({
        text: "one\n2\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\n",
        rejected: [],
      });
      expect([d.moved, d.back, d.roundTrip]).toEqual([true, true, true]);
      expect(d.staged).toBe(
        "diff --git a/src/app.ts b/src/app.ts\nindex 83db48f..bf269f4 100644\n--- a/src/app.ts\n+++ b/src/app.ts\n@@ -8,3 +8,4 @@\n eight\n nine\n ten\n+eleven\n",
      );
      expect(d.stagedApplied).toBe("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\n");
      expect(d.fromText).toEqual(["--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n", "a\nB\n"]);
      expect(d.rejected).toEqual({ text: "1\nY\n3\n", rejected: [0] });
      expect(d.errors.map((e: string) => e.split(":")[0])).toEqual([
        "SyntaxError",
        "TypeError",
        "RangeError",
        "RangeError",
        "TypeError",
        "TypeError",
      ]);

      await app.eval("__SK_TEST_STD__.startDiffs(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.diffsResult)"));
      }
      const a = JSON.parse(raw);
      expect(a.failed).toBeUndefined();
      expect(a.result).toEqual({
        count: 100,
        first: [250, 251, 250, 251],
        part: ["compute", "changed"],
        sameAsSync: true,
        rejected: "TypeError",
      });
    },
    T,
  );

  test(
    "skathi:collections keeps bits, numbers and keyed values",
    async () => {
      const app = await start();
      const c = JSON.parse(String(await app.eval("__SK_TEST_STD__.collections()")));
      expect(c.bits).toEqual([130, 65, [0, 2, 4], [0, 1, 2, 3, 4, 5, 64, 129], true, false, false]);
      expect(c.lowAfterClear).toBe(6);
      expect(c.outside).toStartWith("RangeError: BitSet.set: index 130 is outside 0..130");
      expect(c.mismatch).toStartWith("RangeError: BitSet.and:");
      expect(c.bloom).toEqual([true, true, false, false, 3]);
      expect(c.badKey).toStartWith("TypeError");
      expect(c.ring).toEqual([[null, null, null, 1], [4, 5, 6], 4, 6, null, true, 4, 2]);
      expect(c.sets).toEqual([[true, true, false], 4, true, false, true]);
      expect(c.trie).toEqual([
        4,
        [["/users", 2]],
        ["/", "/about", "/users", "/zeta"],
        true,
        5,
        "/users",
        null,
        [true, false],
        6,
      ]);
      expect(c.intervals).toEqual([["a", "b", "b2", "c"], ["b2"], [true, false], 3, []]);
      expect(c.sameObject).toEqual([true, false, true]);
      expect(c.backwards).toStartWith("RangeError");
      expect(c.globals).toEqual([]);
    },
    T,
  );

  test(
    "skathi:network reads the connection from the OS",
    async () => {
      const app = await start();
      const n = JSON.parse(String(await app.eval("__SK_TEST_STD__.network()")));
      expect(n.sameOnline).toBe(true);
      expect(n.target).toBe(true);
      expect(["full", "limited", "portal", "none", "unknown"]).toContain(n.internet);
      expect(["ethernet", "wifi", "cellular", "other", "none", "unknown"]).toContain(n.type);
      for (const flag of [n.metered, n.roaming, n.overDataLimit]) expect([true, false, null]).toContain(flag);
      expect(n.wrapped).toBe(1);
      expect(n.connection).toEqual({ type: n.type, same: true, instance: true, target: true, measured: [] });
    },
    T,
  );

  test(
    "navigator.connection follows skathi:network and fires change when the type changes",
    async () => {
      const app = await launch({
        app: "packages/test/fixtures/std/src/main.ts",
        window: [420, 300],
        label: "std-network",
        env: { SK_NETWORK: "fake" },
      });
      apps.push(app);
      await app.locator("#ready").waitFor();
      await app.eval("__SK_TEST_STD__.startConnectionChange(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.connectionResult)"));
      }
      const c = JSON.parse(raw);
      expect(c.failed).toBeUndefined();
      expect(c.steps).toEqual([
        ["wifi", true],
        ["wifi", true],
        ["none", true],
      ]);
      expect(c.log).toEqual([
        "network:change",
        "connection:wifi",
        "onchange",
        "network:change",
        "network:offline",
        "network:change",
        "connection:none",
        "onchange",
      ]);
      expect([c.metered, c.onLine]).toEqual([true, false]);
    },
    T,
  );

  test(
    "skathi:memory frees on demand, and nothing reads memory a freed buffer gave back",
    async () => {
      const app = await start();
      const m = JSON.parse(String(await app.eval("__SK_TEST_STD__.memory()")));
      const b = m.buffers;
      expect([b.zeroed, b.before, b.after, b.again]).toEqual([true, [16, 16, false], [0, 0, true, 0], "ok"]);
      expect([b.aligned, b.empty, b.transferred]).toEqual(["0", 0, [true, 8]]);
      expect(b.errors).toEqual(["RangeError", "RangeError", "TypeError"]);

      const r = m.binary;
      expect([r.header, r.copied, r.crc]).toEqual([{ magic: 0x01020304, count: 7 }, [4, 3, 2, 1, 7, 0, 0, 0], true]);
      expect([r.readFreed, r.newFreed, r.crcFreed]).toEqual(Array(3).fill("InvalidStateError"));

      const v = m.views;
      expect(v.uv).toEqual([0.5, 1, 0.25, 0.75, 1, 2]);
      expect(v.interleaved).toEqual(v.uv);
      expect([v.untouched, v.bytes]).toEqual([
        [0, 0, 0],
        [1, 255, 0, 0],
      ]);
      expect(v.errors).toEqual(["RangeError", "RangeError", "RangeError", "RangeError"]);
      expect([v.midway, v.afterFree, v.onFreed]).toEqual([
        "InvalidStateError",
        ["TypeError", "TypeError", "InvalidStateError"],
        "InvalidStateError",
      ]);

      // A write through a typed array over a detached buffer goes nowhere, as the language has it.
      expect(m.gpu).toEqual({ written: [1, 2, 3, 0.5], afterFree: ["ok", 0], onFreed: "InvalidStateError" });

      const a = m.arenas;
      expect([a.used, a.aligned, a.freed]).toEqual([108, "0", [true, true, 0]]);
      expect([a.notReused, a.kept, a.zeroed]).toEqual([true, [1, 2, 3, 4], true]);

      const f = m.ffi;
      expect([f.copied, f.back]).toEqual([
        [5, 6, 7, 8],
        [5, 6, 7, 8],
      ]);
      expect([f.freedDuringCall, f.sortedInPlace]).toEqual([true, [4, 3, 2, 1]]);
      expect(f.keptUnderView).toEqual([9, 8, 7, 6]);
      expect([f.freedArgument, f.freedPointer]).toEqual(["InvalidStateError", "InvalidStateError"]);

      await app.eval("__SK_TEST_STD__.startMemoryWorkers(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.memoryWorkersResult)"));
      }
      const w = JSON.parse(raw);
      expect(w.failed).toBeUndefined();
      expect(w.result).toEqual({
        during: [0, false, 0],
        twice: "InvalidStateError",
        readDuring: "InvalidStateError",
        after: [4, 3, 2, 1],
        earlier: 0,
        droppedStaysFreed: [true, 0],
      });
      expect(app.child.exitCode).toBeNull();
    },
    T,
  );

  test(
    "skathi:image decodes, resizes and encodes on workers, from bytes, a NativeBuffer or a stream",
    async () => {
      const app = await start();
      await app.eval("__SK_TEST_STD__.startImage(); 1");
      let raw = "null";
      for (const end = Date.now() + 20_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.imageResult)"));
      }
      const i = JSON.parse(raw);
      expect(i.failed).toBeUndefined();
      expect(i.result).toEqual({
        streamed: [false, true, "image/png", 1, false],
        decoded: [true, true, 32, true],
        thumb: [2, 1, "image/png"],
        small: [2, 1, 8],
        jpeg: [0xff, 0xd8, 2, 1],
        bitmap: [4, 2, "255,0,0,255"],
        more: [true, true, "255,0,0,255,0,0,255,255"],
        supported: [true, false, true, true],
        errors: {
          truncated: "EncodingError",
          wrongType: "EncodingError",
          noFrame: "RangeError",
          enlarge: "RangeError",
          unwritable: "NotSupportedError",
          closed: "InvalidStateError",
          aborted: "AbortError",
          freed: "InvalidStateError",
        },
      });
      expect(app.child.exitCode).toBeNull();
    },
    T,
  );

  test(
    "skathi:process starts a listed program without a shell, streams its output and refuses the rest",
    async () => {
      const app = await start();
      await app.eval("__SK_TEST_STD__.startProcess(); 1");
      let raw = "null";
      for (const end = Date.now() + 30_000; raw === "null" && Date.now() < end; await Bun.sleep(50)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.processResult)"));
      }
      const p = JSON.parse(raw);
      expect(p.failed).toBeUndefined();
      const r = p.result;
      expect(r.same).toBe(true);
      expect(r.exit).toEqual([3, null, "to stderr"]);
      expect(r.roundTrip).toEqual({ length: 4 * 1024 * 1024, sum: 4 * 1024 * 1024 });
      expect(r.killed).toBeNull();
      expect([r.unlisted, r.path, r.relative]).toEqual(["NotAllowedError", "NotAllowedError", "NotAllowedError"]);
    },
    T,
  );

  test(
    "skathi:process hands a flood of output to JS frame after frame, under the engine's lock",
    async () => {
      const app = await start();
      const size = 48 * 1024 * 1024;
      await app.eval(`__SK_TEST_STD__.startFlood(${size}); 1`);
      let raw = "null";
      for (const end = Date.now() + 25_000; raw === "null" && Date.now() < end; await Bun.sleep(100)) {
        raw = String(await app.eval("String(__SK_TEST_STD__.floodResult)"));
      }
      expect(JSON.parse(raw)).toEqual({ length: size });
    },
    T,
  );

  test(
    "a child of skathi:process does not outlive the app",
    async () => {
      const app = await start();
      const pid = Number(await app.eval("__SK_TEST_STD__.lingering()"));
      started.set(pid, /^bun\.exe$/i);
      const alive = () => {
        try {
          process.kill(pid, 0);
          return true;
        } catch {
          return false;
        }
      };
      expect(alive()).toBe(true);
      void app.eval("__SK_TEST_STD__.app.quit(); 1").catch(() => {});
      expect(await ended(app)).toBe(0);
      const deadline = Date.now() + 5_000;
      while (alive() && Date.now() < deadline) await Bun.sleep(50);
      expect(alive()).toBe(false);
      started.delete(pid);
    },
    T,
  );

  test(
    "app.quit({ exitCode }) fires pagehide and ends the process with that code",
    async () => {
      const app = await start();
      await app.eval(`addEventListener("pagehide", () => console.log("[std-spec] pagehide")); "ok"`);
      void app.eval("__SK_TEST_STD__.app.quit({ exitCode: 7 }); 1").catch(() => {});
      expect(await ended(app)).toBe(7);
      const log = readFileSync(app.hostLog, "utf8");
      expect(log.split("[std-spec] pagehide").length - 1).toBe(1);
    },
    T,
  );

  test(
    "app.restart({ args }) starts the program again, then ends this process",
    async () => {
      const app = await start();
      // A host given this argument prints what it is and exits, so nothing is left running.
      void app.eval(`__SK_TEST_STD__.app.restart({ args: ["--sk-host-info"] }); 1`).catch(() => {});
      expect(await ended(app)).toBe(0);
      const pid = /app\.restart: started process (\d+)/.exec(readFileSync(app.hostLog, "utf8"))?.[1];
      expect(pid).toBeDefined();
      started.set(Number(pid), new RegExp(`^sk-app-sktest-${process.pid}\\.exe$`, "i"));
    },
    T,
  );
});
