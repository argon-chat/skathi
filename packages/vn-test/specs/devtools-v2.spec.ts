import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DEFAULT_EXE, launch } from "../src/index";

const T = 40_000;

const app = await launch({
  app: "hello",
  window: [820, 600],
  label: "dt2",
  env: { VN_DEVTOOLS_RAISE: "off" },
});

const strays: ChildProcess[] = [];
const scratch: string[] = [];

afterAll(async () => {
  for (const child of strays) {
    try {
      child.kill();
    } catch {}
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

async function until<T>(what: string, probe: () => Promise<T>, ms = 15_000): Promise<NonNullable<T>> {
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

describe("the inspector is embedded in the executable", () => {
  test(
    "`--devtools` boots the whole UI with an EMPTY dist slot",
    async () => {
      const slot = mkdtempSync(join(tmpdir(), "vn-embedded-"));
      scratch.push(slot);
      bunExpect(existsSync(join(slot, "manifest.json"))).toBe(false);

      // The repo's own exe rather than the staged copy `launch` made: this host is started by hand,
      // so nothing needs to be able to delete it mid-run.
      const child = spawn(DEFAULT_EXE, ["--devtools", "--target", app.slotDir], {
        env: { ...process.env, VN_DIST: slot, VN_WINDOW: "700x520", VN_DEVTOOLS_RAISE: "off" },
        stdio: "ignore",
        detached: false,
      });
      strays.push(child);

      const shake = await until("the embedded inspector's handshake", async () =>
        existsSync(join(slot, "devtools.json")) ? handshake(slot) : null,
      );
      const tree = await until("the inspector's own tree", async () => {
        const t = await command(shake.port, shake.token, "tree", { depth: 3 });
        return t && t.children && t.children.length ? t : null;
      });

      const flat = JSON.stringify(tree);
      bunExpect(flat).toContain("titlebar");
      bunExpect(flat).toContain("toolbar");

      const attached = await until("the inspector to attach", async () => {
        const q = await command(shake.port, shake.token, "query", { selector: ".what .slot" });
        return q.count === 1 ? q : null;
      });
      bunExpect(attached.nodes[0].text).toBe(app.slotDir.split(/[\\/]/).pop());
    },
    T,
  );
});

describe("F12", () => {
  test(
    "opens an inspector for the window it was pressed in, and never a second",
    async () => {
      const before = await app.send("windows");
      bunExpect(before.inspectors).toEqual([]);
      const mainWindow = before.windows.find((w: any) => w.kind === "main");
      bunExpect(mainWindow).toBeDefined();

      // A REAL keystroke through gpui's own key path — the same one a person's F12 takes.
      await app.press("F12");

      const one = await until("the inspector to start", async () => {
        const w = await app.send("windows");
        return w.inspectors.length === 1 ? w : null;
      });
      bunExpect(one.inspectors[0].window).toBe(mainWindow.id);
      const pid = one.inspectors[0].pid;

      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => /F12: opened the inspector .*did not take focus/.test(e.text))).toBe(true);

      await app.press("F12");
      await sleep(250);
      const raisings = async () => (await app.logs(0)).entries.filter((e) => /F12: raising/.test(e.text)).length;
      bunExpect(await raisings()).toBe(0);

      await sleep(700);
      await app.press("F12");
      await sleep(1500);
      const two = await app.send("windows");
      bunExpect(two.inspectors.length).toBe(1);
      bunExpect(two.inspectors[0].pid).toBe(pid);
      bunExpect(await raisings()).toBe(1);
    },
    T,
  );

  test(
    "the inspector it opened is attached to this app and reads its tree",
    async () => {
      const windows = await app.send("windows");
      const pid = windows.inspectors[0].pid;
      const slot = join(tmpdir(), "vue-native", "devtools", `${windows.pid}-w${windows.windows[0].id}`);
      const shake = await until("the child's handshake", async () =>
        existsSync(join(slot, "devtools.json")) ? handshake(slot) : null,
      );
      bunExpect(shake.pid).toBe(pid);

      const chip = await until("the inspector to report the target", async () => {
        const q = await command(shake.port, shake.token, "query", { selector: ".what .pid" });
        return q.count === 1 && q.nodes[0].text ? q.nodes[0].text : null;
      });
      bunExpect(chip).toBe(`pid ${windows.pid}`);
    },
    T,
  );

  test(
    "`VN_DEVTOOLS_KEY=off` declines the key and leaves the protocol alone",
    async () => {
      // It exists for a harness driving an app on a machine somebody is using, where a stray F12
      // opening a window is noise — so the port must stay.
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
        bunExpect(entries.some((e) => /VN_DEVTOOLS_KEY=off.*inspector is not opened/.test(e.text))).toBe(true);
        bunExpect((await optOut.send("ping")).pid).toBeGreaterThan(0);
      } finally {
        await optOut.close();
      }
    },
    T,
  );
});

describe("the push channel", () => {
  test(
    "a log line arrives on the socket with nobody polling",
    async () => {
      const shake = handshake(app.slotDir);
      const url = `ws://127.0.0.1:${shake.port}/?token=${encodeURIComponent(shake.token)}&since=0`;
      const socket = new WebSocket(url);
      const seen: any[] = [];
      socket.onmessage = (event: MessageEvent) => seen.push(JSON.parse(String(event.data)));
      try {
        await until("the socket to open", async () => socket.readyState === 1);
        // The handshake message names the protocol and the cursor it will stream from, so a client
        // knows what it is attached to before anything happens.
        const hello = await until("hello", async () => seen.find((m) => m.t === "hello"));
        bunExpect(hello.pid).toBe(app.pid);
        bunExpect(typeof hello.logCursor).toBe("number");

        const marker = `vn-push-${Date.now()}`;
        await app.eval(`console.log(${JSON.stringify(marker)})`);

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

        const perf = await until("a perf sample", async () => seen.find((m) => m.t === "perf"), 6000);
        bunExpect(perf.fps).toBeGreaterThan(0);
        bunExpect(perf.nodes).toBeGreaterThan(0);
      } finally {
        socket.close();
      }
    },
    T,
  );

  test(
    "a bad token is refused before the upgrade",
    async () => {
      const shake = handshake(app.slotDir);
      const socket = new WebSocket(`ws://127.0.0.1:${shake.port}/?token=nope`);
      const closed = await new Promise<boolean>((r) => {
        socket.onopen = () => r(false);
        socket.onerror = () => r(true);
        socket.onclose = () => r(true);
        setTimeout(() => r(false), 4000);
      });
      bunExpect(closed).toBe(true);
    },
    T,
  );
});

describe("per-window addressing", () => {
  test(
    "a pop-out is a window the protocol can name, and answer about",
    async () => {
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

        // `tree` with a window and nothing else is rooted at what THAT window paints — the
        // popped-out subtree, not the document.
        const popTree = await two.send("tree", { window: pop.id, depth: 3 });
        bunExpect(popTree.id).toBe(pop.root);

        const picked = await two.send("pick", { window: pop.id });
        bunExpect(picked.window).toBe(pop.id);
        bunExpect(picked.viewport.w).toBe(pop.viewport.w);

        const hit = await two.send("mouse", {
          window: pop.id,
          x: Math.round(pop.viewport.w / 2),
          y: Math.round(pop.viewport.h / 2),
          kind: "move",
        });
        bunExpect(hit.x).toBeGreaterThan(0);

        const outlined = await two.send("highlight", {
          window: pop.id,
          node: pop.root,
          ttlMs: 1500,
        });
        bunExpect(outlined.painted).toBe(true);

        const shot = await two.send("screenshot", { window: pop.id });
        bunExpect(shot.width).toBeGreaterThan(0);
        bunExpect(shot.width).not.toBe((await two.send("screenshot", { window: main.id })).width);
      } finally {
        await two.close();
      }
    },
    T,
  );
});

describe("what the inspector found in the framework", () => {
  // Both are the ENGINE's, both were fixed there rather than worked around in the app, and both
  // are pinned here because the next app to hit them will not be a devtools panel.

  test(
    "a wheel event reaches JS with the DOM's sign, not the platform's",
    async () => {
      // gpui carries the platform's sign: on Windows the wheel's distance is POSITIVE when it turns
      // away from you, i.e. a scroll UP.
      await app.eval(`
      globalThis.__wheel = null;
      var el = document.querySelector(".card") || document.querySelector("div");
      el.addEventListener("wheel", function (e) { globalThis.__wheel = e.deltaY; });
      globalThis.__wheelEl = el;
      0
    `);
      const box = await app
        .locator(".card")
        .boundingBox()
        .catch(() => null);
      const at = box ? { x: Math.round(box.x + box.w / 2), y: Math.round(box.y + box.h / 2) } : { x: 40, y: 40 };

      await app.send("wheel", { ...at, dy: 120 });
      bunExpect(await app.eval("globalThis.__wheel")).toBeGreaterThan(0);

      await app.send("wheel", { ...at, dy: -120 });
      bunExpect(await app.eval("globalThis.__wheel")).toBeLessThan(0);
    },
    T,
  );

  test(
    "a newline inside a text node is white space, not a line break",
    async () => {
      const height = async (text: string) => {
        await app.eval(`document.querySelector(".mark").textContent = ${JSON.stringify(text)}; 0`);
        await sleep(200);
        const found = await app.query(".mark");
        return found.nodes[0]!.bounds!.h;
      };
      const one = await height("alpha beta");
      bunExpect(one).toBeGreaterThan(0);
      const withNewlines = await height("alpha\n\n\nbeta");
      bunExpect(withNewlines).toBe(one);
      // …and the collapsing is a COLLAPSE, not a deletion: the words stay apart, and `textContent`
      // still reports the RAW string (the DOM does not collapse — only the rendering does).
      bunExpect(await app.locator(".mark").textContent()).toBe("alpha\n\n\nbeta");
    },
    T,
  );

  test(
    "console format specifiers are applied, and `%c` becomes a style run",
    async () => {
      // Before it, `%c` reached the ring literally and the CSS argument — which apps write across
      // several lines — was appended as message text.
      const marker = `fmt-${Date.now()}`;
      await app.eval(`console.log("%cbadge%c ${marker} %s/%d", "color:#fff;background:#00BCD4", "", "x", 42); 0`);
      const line = await until(
        "the formatted line",
        async () => {
          const { entries } = await app.logs(0);
          return entries.find((e) => e.text.includes(marker));
        },
        6000,
      );
      bunExpect(line.text).toBe(`badge ${marker} x/42`);
      bunExpect(line.text).not.toContain("%c");
      bunExpect(line.text).not.toContain("00BCD4");
      const styles = (line as any).styles;
      bunExpect(Array.isArray(styles)).toBe(true);
      bunExpect(styles[0].at).toBe(0);
      bunExpect(styles[0].css).toContain("00BCD4");
      bunExpect(styles[1].at).toBe(5);

      // `%%` is a literal percent — but ONLY when the formatter runs at all, which the Console
      // Standard says happens for a call with MORE THAN ONE argument.
      await app.eval(`console.log("100%% sure %s", "today"); console.log("plain", 1, {a:2}); 0`);
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text === "100% sure today")).toBe(true);
      bunExpect(entries.some((e) => e.text === 'plain 1 {"a":2}')).toBe(true);
    },
    T,
  );

  test(
    "an Error in the ring is its header and stack, not `{}`",
    async () => {
      const marker = `err-${Date.now()}`;
      await app.eval(
        `console.warn("%cwarn%c ${marker}:", "background:#c0392b", "", new TypeError("bad thing")); ` +
          `console.log("${marker}-nested", { cause: new RangeError("deep"), n: 2n }); 0`,
      );
      const line = await until(
        "the error line",
        async () => (await app.logs(0)).entries.find((e) => e.text.includes(`${marker}:`)),
        6000,
      );
      bunExpect(line.text.startsWith(`warn ${marker}: TypeError: bad thing`)).toBe(true);
      bunExpect(line.text).not.toContain("{}");
      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text === `${marker}-nested {"cause":"RangeError: deep","n":"2n"}`)).toBe(true);
    },
    T,
  );
});

describe("the new commands", () => {
  test(
    "`perf` reports the rolling frame window with no VN_PERF anywhere",
    async () => {
      const perf = await app.send("perf");
      bunExpect(perf.frames).toBeGreaterThan(10);
      bunExpect(perf.fps).toBeGreaterThan(1);
      bunExpect(perf.p50Ms).toBeGreaterThan(0);
      bunExpect(perf.p95Ms).toBeGreaterThanOrEqual(perf.p50Ms);
      bunExpect(perf.nodes).toBeGreaterThan(0);
      bunExpect(perf.samples.length).toBeGreaterThan(0);
      // Each frame's work sits beside its interval and is part of it.
      bunExpect(perf.work.length).toBe(perf.samples.length);
      bunExpect(perf.flags.length).toBe(perf.samples.length);
      bunExpect(perf.workP50Ms).toBeGreaterThan(0);
      bunExpect(perf.workP50Ms).toBeLessThanOrEqual(perf.p50Ms);
      bunExpect(perf.drewShare).toBeGreaterThan(0);
      bunExpect(perf.bootMs).toBeGreaterThan(0);
      bunExpect(perf.phases).toBe(false);
    },
    T,
  );

  test(
    "`storage` reads and writes the app's own localStorage",
    async () => {
      const key = `vn.spec.${Date.now()}`;
      await app.send("storage", { area: "local", op: "set", key, value: "written by the panel" });
      const listed = await app.send("storage", { area: "local", op: "list" });
      bunExpect(listed.entries.some((e: any) => e.key === key)).toBe(true);

      // It went through the app's OWN JS, so the app sees it too — which is the whole reason the
      // command is an `eval` rather than a read of the disk.
      bunExpect(await app.eval(`localStorage.getItem(${JSON.stringify(key)})`)).toBe("written by the panel");

      await app.send("storage", { area: "local", op: "remove", key });
      bunExpect(await app.eval(`localStorage.getItem(${JSON.stringify(key)})`)).toBe(null);
    },
    T,
  );

  test(
    "`net` records what the app fetched, including its failures",
    async () => {
      // Port 9 is `discard`; nothing is listening, so this is a connect failure — which is the entry
      // a network panel most needs to show.
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
    },
    T,
  );

  test(
    "`windows` names every window and every inspector",
    async () => {
      const w = await app.send("windows");
      bunExpect(w.pid).toBe(app.pid);
      bunExpect(w.windows.length).toBeGreaterThan(0);
      bunExpect(w.windows[0].kind).toBe("main");
      bunExpect(w.windows[0].root).toBe(0);
      bunExpect(typeof w.windows[0].focused).toBe("boolean");
    },
    T,
  );

  test(
    "nothing in the run was a JS error",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
