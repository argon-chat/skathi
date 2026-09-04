// devtools v2 — the three claims, held down from outside the process.
//
//   bun test packages/vn-test/specs/devtools-v2.spec.ts
//
// 1. THE INSPECTOR IS IN THE BINARY. A host started with `--devtools` and a
//    VN_DIST pointing at an EMPTY directory boots the whole inspector: no
//    dist/, no manifest, no vendor bundle, nothing on disk at all. That is the
//    only way to prove `include_str!` rather than "it happened to find a build".
//
// 2. F12 OPENS ONE. In whatever window the key landed in, without taking focus,
//    and pressing it again raises the one that exists instead of opening a
//    second.
//
// 3. THE HOST PUSHES. A `console.log` in the app arrives on a WebSocket the
//    test opened, with nobody polling anything.
//
// …plus the per-window addressing the pop-out spike needed: with two windows
// off one tree, `windows`, `tree`, `pick`, `highlight` and `screenshot` all
// mean "that one" instead of "the primary one".
//
// Nothing here focuses a window or moves the real cursor. `VN_DEVTOOLS_RAISE=off`
// is set on every host that could be asked to come forward: the second-F12 path
// is exercised in full (no duplicate process, the log line) with the one step
// that would steal the foreground turned off, because a spec must not take the
// screen away from whoever is using the machine.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DEFAULT_EXE, expect, launch } from "../src/index";

const T = 40_000;

const app = await launch({
  app: "hello",
  window: [820, 600],
  label: "dt2",
  env: { VN_DEVTOOLS_RAISE: "off" },
});

/** Hosts this file started by hand (not through `launch`). */
const strays: ChildProcess[] = [];
const scratch: string[] = [];

afterAll(async () => {
  for (const child of strays) {
    try {
      child.kill();
    } catch {
      /* already gone */
    }
  }
  for (const dir of scratch) {
    try {
      rmSync(dir, { recursive: true, force: true });
    } catch {
      /* Windows can hold a view of it for a beat */
    }
  }
  await app.close();
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(what: string, probe: () => Promise<T>, ms = 15_000): Promise<T> {
  const deadline = Date.now() + ms;
  let last: unknown;
  for (;;) {
    const v = await probe().catch((e) => {
      last = e;
      return undefined as T;
    });
    if (v) return v;
    if (v !== undefined) last = v;
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
    }
    await sleep(120);
  }
}

/** One NDJSON command against an arbitrary handshake — the raw wire. */
function command(port: number, token: string, cmd: string, args: Record<string, unknown> = {}) {
  return new Promise<any>((resolveReply, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error(`${cmd}: no reply in 10s`));
    }, 10_000);
    socket.on("connect", () => socket.write(`${JSON.stringify({ token, cmd, ...args })}\n`));
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      const nl = buffer.indexOf("\n");
      if (nl < 0) return;
      clearTimeout(timer);
      socket.destroy();
      const reply = JSON.parse(buffer.slice(0, nl));
      reply.ok ? resolveReply(reply.data) : reject(new Error(reply.error));
    });
    socket.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
  });
}

function handshake(slot: string): { port: number; pid: number; token: string } {
  return JSON.parse(readFileSync(join(slot, "devtools.json"), "utf8"));
}

// ---------------------------------------------------------------------------

describe("the inspector is embedded in the executable", () => {
  test("`--devtools` boots the whole UI with an EMPTY dist slot", async () => {
    // The point of the empty directory: there is no manifest, no vendor.js, no
    // module scripts and no app.prod.js anywhere the host could read. If a tree
    // appears, it was `include_str!`d into the exe.
    const slot = mkdtempSync(join(tmpdir(), "vn-embedded-"));
    scratch.push(slot);
    bunExpect(existsSync(join(slot, "manifest.json"))).toBe(false);

    // The repo's own exe rather than the staged copy `launch` made: this host
    // is started by hand, so nothing needs to be able to delete it mid-run.
    const child = spawn(
      DEFAULT_EXE,
      ["--devtools", "--target", app.slotDir],
      {
        env: { ...process.env, VN_DIST: slot, VN_WINDOW: "700x520", VN_DEVTOOLS_RAISE: "off" },
        stdio: "ignore",
        detached: false,
      },
    );
    strays.push(child);

    const shake = await until("the embedded inspector's handshake", async () =>
      existsSync(join(slot, "devtools.json")) ? handshake(slot) : null,
    );
    const tree = await until("the inspector's own tree", async () => {
      const t = await command(shake.port, shake.token, "tree", { depth: 3 });
      return t && t.children && t.children.length ? t : null;
    });

    // The inspector's own markup, rendered out of the binary: the shell root,
    // its titlebar, and the tab strip.
    const flat = JSON.stringify(tree);
    bunExpect(flat).toContain("titlebar");
    bunExpect(flat).toContain("toolbar");

    // …and it attached to the app we pointed it at, which is the other half of
    // "it works with no dist": discovery, the wire and the UI all came out of
    // the same executable.
    const attached = await until("the inspector to attach", async () => {
      const q = await command(shake.port, shake.token, "query", { selector: ".what .slot" });
      return q.count === 1 ? q : null;
    });
    bunExpect(attached.nodes[0].text).toBe(app.slotDir.split(/[\\/]/).pop());
  }, T);
});

describe("F12", () => {
  test("opens an inspector for the window it was pressed in, and never a second", async () => {
    const before = await app.send("windows");
    bunExpect(before.inspectors).toEqual([]);
    const mainWindow = before.windows.find((w: any) => w.kind === "main");
    bunExpect(mainWindow).toBeDefined();

    // A REAL keystroke through gpui's own key path — the same one a person's
    // F12 takes. `key` with no selector goes to whatever has focus, and the
    // interceptor runs before every element listener (`inspector.rs`).
    await app.press("F12");

    const one = await until("the inspector to start", async () => {
      const w = await app.send("windows");
      return w.inspectors.length === 1 ? w : null;
    });
    bunExpect(one.inspectors[0].window).toBe(mainWindow.id);
    const pid = one.inspectors[0].pid;

    // It said so, and it said it did not take focus.
    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => /F12: opened the inspector .*did not take focus/.test(e.text))).toBe(
      true,
    );

    // A REPEAT — holding the key auto-repeats at ~30 Hz on Windows and every
    // repeat is a real `KeyDown` — is ignored inside the 500 ms debounce
    // (`inspector.rs`, `F12_DEBOUNCE`), so it neither opens nor raises.
    await app.press("F12");
    await sleep(250);
    const raisings = async () =>
      (await app.logs(0)).entries.filter((e) => /F12: raising/.test(e.text)).length;
    bunExpect(await raisings()).toBe(0);

    // Past the window, a second press RAISES rather than duplicating.
    await sleep(700);
    await app.press("F12");
    await sleep(1500);
    const two = await app.send("windows");
    bunExpect(two.inspectors.length).toBe(1);
    bunExpect(two.inspectors[0].pid).toBe(pid);
    bunExpect(await raisings()).toBe(1);
  }, T);

  test("the inspector it opened is attached to this app and reads its tree", async () => {
    const windows = await app.send("windows");
    const pid = windows.inspectors[0].pid;
    const slot = join(tmpdir(), "vue-native", "devtools", `${windows.pid}-w${windows.windows[0].id}`);
    const shake = await until("the child's handshake", async () =>
      existsSync(join(slot, "devtools.json")) ? handshake(slot) : null,
    );
    bunExpect(shake.pid).toBe(pid);

    // The breadcrumb strip is the inspector's own; what it is ATTACHED to is
    // read out of its titlebar, which only says a slot once `ping` came back
    // from the other process.
    const chip = await until("the inspector to report the target", async () => {
      const q = await command(shake.port, shake.token, "query", { selector: ".what .pid" });
      return q.count === 1 && q.nodes[0].text ? q.nodes[0].text : null;
    });
    bunExpect(chip).toBe(`pid ${windows.pid}`);
  }, T);

  test("`VN_DEVTOOLS_KEY=off` declines the key and leaves the protocol alone", async () => {
    // The KEY gate, which since m22 is a different thing from the devtools
    // switch (docs/DEVTOOLS.md "Two gates"). It exists for a harness driving an
    // app on a machine somebody is using, where a stray F12 opening a window is
    // noise — so the port must stay. The MASTER switch, which takes the port
    // with it, is `appDevtools.disable()` / `"devtools": false`, and it has a
    // file of its own: devtools-optout.spec.ts.
    const optOut = await launch({
      app: "hello",
      window: [500, 380],
      label: "dt2-optout",
      env: { VN_DEVTOOLS_KEY: "off", VN_DEVTOOLS_RAISE: "off" },
    });
    try {
      await optOut.press("F12");
      await sleep(1500);
      const w = await optOut.send("windows");
      bunExpect(w.inspectors).toEqual([]);
      const { entries } = await optOut.logs(0);
      bunExpect(
        entries.some((e) => /VN_DEVTOOLS_KEY=off.*inspector is not opened/.test(e.text)),
      ).toBe(true);
      // …and the PROTOCOL is untouched: the port still answers, which is what
      // keeps a harness from being locked out by a key preference.
      bunExpect((await optOut.send("ping")).pid).toBeGreaterThan(0);
    } finally {
      await optOut.close();
    }
  }, T);
});

describe("the push channel", () => {
  test("a log line arrives on the socket with nobody polling", async () => {
    const shake = handshake(app.slotDir);
    const url = `ws://127.0.0.1:${shake.port}/?token=${encodeURIComponent(shake.token)}&since=0`;
    const socket = new WebSocket(url);
    const seen: any[] = [];
    socket.onmessage = (event: MessageEvent) => seen.push(JSON.parse(String(event.data)));
    try {
      await until("the socket to open", async () => socket.readyState === 1);
      // The handshake message names the protocol and the cursor it will stream
      // from, so a client knows what it is attached to before anything happens.
      const hello = await until("hello", async () => seen.find((m) => m.t === "hello"));
      bunExpect(hello.pid).toBe(app.pid);
      bunExpect(typeof hello.logCursor).toBe("number");

      const marker = `vn-push-${Date.now()}`;
      await app.eval(`console.log(${JSON.stringify(marker)})`);

      // NO POLL between here and the assertion. If this passes, the host wrote
      // it out of `pump_frame` (`devtools::ws::broadcast_frame`).
      const line = await until(
        "the pushed log line",
        async () =>
          seen
            .filter((m) => m.t === "logs")
            .flatMap((m) => m.entries || [])
            .find((e: any) => e.text.includes(marker)),
        6000,
      );
      bunExpect(line.kind).toBe("log");

      // Frame stats ride the same socket, four times a second.
      const perf = await until("a perf sample", async () => seen.find((m) => m.t === "perf"), 6000);
      bunExpect(perf.fps).toBeGreaterThan(0);
      bunExpect(perf.nodes).toBeGreaterThan(0);
    } finally {
      socket.close();
    }
  }, T);

  test("a bad token is refused before the upgrade", async () => {
    const shake = handshake(app.slotDir);
    const socket = new WebSocket(`ws://127.0.0.1:${shake.port}/?token=nope`);
    const closed = await new Promise<boolean>((r) => {
      socket.onopen = () => r(false);
      socket.onerror = () => r(true);
      socket.onclose = () => r(true);
      setTimeout(() => r(false), 4000);
    });
    bunExpect(closed).toBe(true);
  }, T);
});

describe("per-window addressing", () => {
  test("a pop-out is a window the protocol can name, and answer about", async () => {
    const two = await launch({
      app: "popout",
      window: [620, 460],
      label: "dt2-popout",
      env: { VN_DEVTOOLS_RAISE: "off" },
    });
    try {
      const before = await two.send("windows");
      bunExpect(before.windows.length).toBe(1);

      await two.locator(".pop").click();
      const listed = await until("the second window", async () => {
        const w = await two.send("windows");
        return w.windows.length === 2 ? w : null;
      });

      const main = listed.windows.find((w: any) => w.kind === "main");
      const pop = listed.windows.find((w: any) => w.kind === "popout");
      bunExpect(pop.root).toBeGreaterThan(0);
      bunExpect(pop.viewport.w).toBeGreaterThan(0);
      bunExpect(pop.viewport.w).not.toBe(main.viewport.w);

      // `tree` with a window and nothing else is rooted at what THAT window
      // paints — the popped-out subtree, not the document.
      const popTree = await two.send("tree", { window: pop.id, depth: 3 });
      bunExpect(popTree.id).toBe(pop.root);

      // `pick` answers FOR a window: its viewport, and its id back.
      const picked = await two.send("pick", { window: pop.id });
      bunExpect(picked.window).toBe(pop.id);
      bunExpect(picked.viewport.w).toBe(pop.viewport.w);

      // A real pointer event, into the pop-out's own `Window`, at ITS
      // coordinates — the gesture `__vn_window_click` was invented for in the
      // spike, now an ordinary `mouse` with a window on it.
      const hit = await two.send("mouse", {
        window: pop.id,
        x: Math.round(pop.viewport.w / 2),
        y: Math.round(pop.viewport.h / 2),
        kind: "move",
      });
      bunExpect(hit.x).toBeGreaterThan(0);

      // The outline is painted BY that window, out of that window's own bounds
      // map — the primary window's rectangles are in a different client space.
      const outlined = await two.send("highlight", {
        window: pop.id,
        node: pop.root,
        ttlMs: 1500,
      });
      bunExpect(outlined.painted).toBe(true);

      // And a capture of that window is that window: a different size.
      const shot = await two.send("screenshot", { window: pop.id });
      bunExpect(shot.width).toBeGreaterThan(0);
      bunExpect(shot.width).not.toBe(
        (await two.send("screenshot", { window: main.id })).width,
      );
    } finally {
      await two.close();
    }
  }, T);
});

describe("what the inspector found in the framework", () => {
  // Two bugs the console surfaced. Both are the ENGINE's, both were fixed
  // there rather than worked around in the app, and both are pinned here
  // because the next app to hit them will not be a devtools panel.

  test("a wheel event reaches JS with the DOM's sign, not the platform's", async () => {
    // gpui carries the platform's sign: on Windows the wheel's distance is
    // POSITIVE when it turns away from you, i.e. a scroll UP. The DOM
    // specifies the mirror image — `deltaY > 0` means scrolling DOWN, because
    // it is "how far the content should move". Until m21 the raw gpui delta
    // was handed to JS, so every `@wheel` handler in every app was inverted.
    await app.eval(`
      globalThis.__wheel = null;
      var el = document.querySelector(".card") || document.querySelector("div");
      el.addEventListener("wheel", function (e) { globalThis.__wheel = e.deltaY; });
      globalThis.__wheelEl = el;
      0
    `);
    const box = await app.locator(".card").boundingBox().catch(() => null);
    const at = box
      ? { x: Math.round(box.x + box.w / 2), y: Math.round(box.y + box.h / 2) }
      : { x: 40, y: 40 };

    await app.send("wheel", { ...at, dy: 120 });
    bunExpect(await app.eval("globalThis.__wheel")).toBeGreaterThan(0);

    await app.send("wheel", { ...at, dy: -120 });
    bunExpect(await app.eval("globalThis.__wheel")).toBeLessThan(0);
  }, T);

  test("a newline inside a text node is white space, not a line break", async () => {
    // CSS: under `white-space: normal` (and `nowrap`) a run of spaces, tabs and
    // newlines collapses to ONE space. The engine used to pass interior runs
    // through untouched and gpui breaks a line at every `\n`, so one text node
    // rendered as several lines — inside a box laid out for one. On the
    // inspector's fixed-height console rows that was glyphs drawn on top of
    // each other, which is how it was found.
    const height = async (text: string) => {
      await app.eval(`document.querySelector(".mark").textContent = ${JSON.stringify(text)}; 0`);
      // Two frames: one for the mutation to land, one for it to be painted
      // (bounds are recorded during prepaint).
      await sleep(200);
      const found = await app.query(".mark");
      return found.nodes[0].bounds!.h;
    };
    const one = await height("alpha beta");
    bunExpect(one).toBeGreaterThan(0);
    const withNewlines = await height("alpha\n\n\nbeta");
    bunExpect(withNewlines).toBe(one);
    // …and the collapsing is a COLLAPSE, not a deletion: the words stay
    // apart, and `textContent` still reports the RAW string (the DOM does
    // not collapse — only the rendering does).
    bunExpect(await app.locator(".mark").textContent()).toBe("alpha\n\n\nbeta");
  }, T);

  test("console format specifiers are applied, and `%c` becomes a style run", async () => {
    // The WHATWG Console Standard's formatter, in the tee
    // (`crates/vn-app/src/devtools/mod.rs`). Before it, `%c` reached the ring
    // literally and the CSS argument — which apps write across several lines —
    // was appended as message text.
    const marker = `fmt-${Date.now()}`;
    await app.eval(
      `console.log("%cbadge%c ${marker} %s/%d", "color:#fff;background:#00BCD4", "", "x", 42); 0`,
    );
    const line = await until(
      "the formatted line",
      async () => {
        const { entries } = await app.logs(0);
        return entries.find((e) => e.text.includes(marker));
      },
      6000,
    );
    // No `%c` in the text, the `%s`/`%d` substituted, the CSS not in the message.
    bunExpect(line.text).toBe(`badge ${marker} x/42`);
    bunExpect(line.text).not.toContain("%c");
    bunExpect(line.text).not.toContain("00BCD4");
    // …and the styling survives as a run the console can paint.
    const styles = (line as any).styles;
    bunExpect(Array.isArray(styles)).toBe(true);
    bunExpect(styles[0].at).toBe(0);
    bunExpect(styles[0].css).toContain("00BCD4");
    bunExpect(styles[1].at).toBe(5);

    // `%%` is a literal percent — but ONLY when the formatter runs at all,
    // which the Console Standard says happens for a call with MORE THAN ONE
    // argument. `console.log("100%% sure")` on its own prints both percent
    // signs, in Chrome and here.
    await app.eval(`console.log("100%% sure %s", "today"); console.log("plain", 1, {a:2}); 0`);
    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => e.text === "100% sure today")).toBe(true);
    bunExpect(entries.some((e) => e.text === 'plain 1 {"a":2}')).toBe(true);
  }, T);
});

describe("the new commands", () => {
  test("`perf` reports the rolling frame window with no VN_PERF anywhere", async () => {
    const perf = await app.send("perf");
    bunExpect(perf.frames).toBeGreaterThan(10);
    bunExpect(perf.fps).toBeGreaterThan(1);
    bunExpect(perf.p50Ms).toBeGreaterThan(0);
    bunExpect(perf.p95Ms).toBeGreaterThanOrEqual(perf.p50Ms);
    bunExpect(perf.nodes).toBeGreaterThan(0);
    bunExpect(perf.samples.length).toBeGreaterThan(0);
    // `boot -> first frame` is a real measurement, not the ring's own zero.
    bunExpect(perf.bootMs).toBeGreaterThan(0);
    // The PHASE breakdown belongs to VN_PERF, which owns those counters.
    bunExpect(perf.phases).toBe(false);
  }, T);

  test("`storage` reads and writes the app's own localStorage", async () => {
    const key = `vn.spec.${Date.now()}`;
    await app.send("storage", { area: "local", op: "set", key, value: "written by the panel" });
    const listed = await app.send("storage", { area: "local", op: "list" });
    bunExpect(listed.entries.some((e: any) => e.key === key)).toBe(true);

    // It went through the app's OWN JS, so the app sees it too — which is the
    // whole reason the command is an `eval` rather than a read of the disk.
    bunExpect(await app.eval(`localStorage.getItem(${JSON.stringify(key)})`)).toBe(
      "written by the panel",
    );

    await app.send("storage", { area: "local", op: "remove", key });
    bunExpect(await app.eval(`localStorage.getItem(${JSON.stringify(key)})`)).toBe(null);
  }, T);

  test("`net` records what the app fetched, including its failures", async () => {
    // Port 9 is `discard`; nothing is listening, so this is a connect failure —
    // which is the entry a network panel most needs to show.
    await app.eval("fetch('http://127.0.0.1:9/spec').then(function(){}, function(){}); 0");
    const entry = await until(
      "the failed fetch",
      async () => {
        const net = await app.send("net", { since: 0 });
        return net.entries.find((e: any) => e.url.includes("/spec"));
      },
      10_000,
    );
    bunExpect(entry.kind).toBe("fetch");
    bunExpect(entry.method).toBe("GET");
    bunExpect(entry.error).toBeTruthy();
    bunExpect(entry.ms).toBeGreaterThanOrEqual(0);
  }, T);

  test("`windows` names every window and every inspector", async () => {
    const w = await app.send("windows");
    bunExpect(w.pid).toBe(app.pid);
    bunExpect(w.windows.length).toBeGreaterThan(0);
    bunExpect(w.windows[0].kind).toBe("main");
    bunExpect(w.windows[0].root).toBe(0);
    bunExpect(typeof w.windows[0].focused).toBe("boolean");
  }, T);

  test("nothing in the run was a JS error", async () => {
    await app.expectNoErrors();
  }, T);
});
