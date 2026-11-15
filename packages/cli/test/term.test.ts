import { describe, expect, test } from "bun:test";
import { glyphs, interactive, lineProgress, spinner, unicodeTerminal } from "../cli/term";

function fakeTty(columns = 80) {
  const writes: string[] = [];
  const stream = Object.assign(Object.create(process.stderr) as NodeJS.WriteStream, {
    isTTY: true,
    columns,
    write: (chunk: string | Uint8Array) => {
      writes.push(typeof chunk === "string" ? chunk : new TextDecoder().decode(chunk));
      return true;
    },
  });
  return { stream, writes, text: () => writes.join("").replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "") };
}

/** The visible screen: `\r` returns to column 0, `\x1b[K` erases to the end of the line. */
function screen(writes: string[]): string[] {
  const rows = [""];
  for (const chunk of writes) {
    for (const part of chunk.split(/(\r|\n|\x1b\[K|\x1b\[[0-9;?]*[A-Za-z])/)) {
      if (part === "\r" || part === "\x1b[K") rows[rows.length - 1] = "";
      else if (part === "\n") rows.push("");
      else if (!part.startsWith("\x1b")) rows[rows.length - 1] += part;
    }
  }
  return rows;
}

describe("the spinner", () => {
  test("a failed phase ends with a cross, not a check, and later calls are no-ops", () => {
    const t = fakeTty();
    const p = spinner(t.stream, false, true);
    p.step("bundle");
    p.fail();
    p.end();
    p.step("never shown");
    expect(screen(t.writes).filter(Boolean)).toEqual([expect.stringMatching(/^✗ bundle \d+ms$/)]);
  });

  test("each frame is one write that redraws the line in place, cut to the terminal's width", () => {
    const t = fakeTty(30);
    const p = spinner(t.stream, false, true);
    p.step("a label far longer than this terminal is wide", 4000);
    for (let i = 0; i < 3000; i++) p.tick();
    const frames = t.writes.filter((w) => w.startsWith("\r") && w.endsWith("\x1b[K") && w.length > 4);
    expect(frames.length).toBeGreaterThan(0);
    for (const f of frames) expect(f.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "").length).toBeLessThanOrEqual(30);
    expect(t.writes.some((w) => w === "\r\x1b[2K")).toBe(false);
    p.end();
  });

  test("console output while it spins lands on its own line above it", () => {
    const t = fakeTty();
    const before = console.warn;
    const p = spinner(t.stream, false, true);
    p.step("bundle");
    expect(console.warn).not.toBe(before);
    p.end();
    expect(console.warn).toBe(before);

    const t2 = fakeTty();
    const logged: unknown[] = [];
    const original = console.log;
    console.log = (...data: unknown[]) => {
      logged.push(...data);
      t2.stream.write(`${data.join(" ")}\n`);
    };
    try {
      const q = spinner(t2.stream, false, true);
      q.step("bundle");
      console.log("from a library");
      q.end();
    } finally {
      console.log = original;
    }
    expect(logged).toEqual(["from a library"]);
    expect(screen(t2.writes).filter(Boolean)).toEqual(["from a library", expect.stringMatching(/^✓ bundle /)]);
  });

  test("a download draws a bar and MiB on a wide terminal", async () => {
    const t = fakeTty(100);
    const p = spinner(t.stream, false, true);
    p.step("standard workload");
    await Bun.sleep(60);
    p.bytes(5 * 2 ** 20, 20 * 2 ** 20);
    expect(t.text()).toMatch(/standard workload ━{5}─{15} 5\.0\/20\.0 MiB/);
    p.end();
  });

  test("signals restore the cursor while it spins, and their handlers go with it", () => {
    const before = process.listenerCount("SIGTERM");
    const t = fakeTty();
    const p = spinner(t.stream, false, true);
    expect(process.listenerCount("SIGINT")).toBeGreaterThan(0);
    expect(process.listenerCount("SIGTERM")).toBe(before + 1);
    p.end();
    expect(process.listenerCount("SIGTERM")).toBe(before);
    expect(t.writes.at(-1)).toBe("\x1b[?25h");
  });
});

test("plain lines mark a failed phase too", () => {
  const out: string[] = [];
  const p = lineProgress((line) => out.push(line), false);
  p.step("host");
  p.end();
  p.step("bundle");
  p.fail();
  expect(out).toEqual([expect.stringMatching(/^✓ host \d+ms$/), expect.stringMatching(/^✗ bundle \d+ms$/)]);
});

test("a redrawn line needs a TTY that is neither TERM=dumb nor a CI log", () => {
  expect(interactive({ isTTY: true }, {})).toBe(true);
  expect(interactive({ isTTY: false }, {})).toBe(false);
  expect(interactive({ isTTY: true }, { TERM: "dumb" })).toBe(false);
  expect(interactive({ isTTY: true }, { CI: "true" })).toBe(false);
  expect(interactive({ isTTY: true }, { CI: "false" })).toBe(true);
});

test("the legacy Windows console gets ASCII glyphs", () => {
  expect(unicodeTerminal({}, "win32")).toBe(false);
  expect(unicodeTerminal({ WT_SESSION: "x" }, "win32")).toBe(true);
  expect(unicodeTerminal({ TERM_PROGRAM: "vscode" }, "win32")).toBe(true);
  expect(unicodeTerminal({}, "linux")).toBe(true);
  expect(glyphs(false).frames.join("") + glyphs(false).ok + glyphs(false).fail).toMatch(/^[\x20-\x7e]+$/);
});
