import { afterAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { arch, homedir, hostname, platform } from "node:os";
import { killTree, launch, type SkApp } from "../src/index";

const T = 60_000;
const apps: SkApp[] = [];
const started: number[] = [];
afterAll(async () => {
  for (const pid of started) killTree(pid);
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
    "skathi:units converts and formats sizes, frequencies and angles",
    async () => {
      const app = await start();
      const u = JSON.parse(String(await app.eval("__SK_TEST_STD__.units()")));
      expect([u.bytes, u.to]).toEqual([1_610_612_736, [1536, 2.5]]);
      expect(u.format.slice(0, 2)).toEqual(["1.5 GB", "1.5 GiB"]);
      expect(u.format[2]).toStartWith("999");
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
      started.push(Number(pid));
    },
    T,
  );
});
