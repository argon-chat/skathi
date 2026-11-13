import { afterAll, expect, test } from "bun:test";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { launch, type VnApp } from "../src/index";

const T = 90_000;
const ENTRY = "packages/vn-test/fixtures/mic/src/main.ts";
const APP_NAME = `vn-test-mic-${process.pid}`;
const appDirs = new Set<string>();
const apps: VnApp[] = [];

afterAll(async () => {
  for (const a of apps) await a.close().catch(() => {});
  for (const dir of appDirs) {
    for (let i = 0; ; i++) {
      try {
        rmSync(dir, { recursive: true, force: true });
        break;
      } catch (e) {
        if (i >= 50) throw e;
        await new Promise((r) => setTimeout(r, 100));
      }
    }
  }
});

async function host(capture: "fake" | "deny", label: string): Promise<VnApp> {
  const app = await launch({
    app: ENTRY,
    window: [420, 200],
    label: `mic-${label}`,
    env: { VN_MEDIA_CAPTURE: capture, VN_APP_NAME: APP_NAME, VN_LOG: "vn_app::microphone=info" },
  });
  apps.push(app);
  return app;
}

async function state(app: VnApp): Promise<string> {
  await app.eval(
    "globalThis.__st = ''; navigator.permissions.query({ name: 'microphone' }).then(s => { globalThis.__st = s.state; }); 0",
  );
  for (let i = 0; i < 100; i++) {
    const s = String(await app.eval("globalThis.__st"));
    if (s) return s;
    await Bun.sleep(20);
  }
  throw new Error("permissions.query never settled");
}

async function open(app: VnApp, answer?: (id: number) => Promise<unknown>): Promise<string> {
  const before = Number(await app.eval("__VN_TEST_MIC__.results.length"));
  await app.eval("__VN_TEST_MIC__.open(), 0");
  if (answer) {
    const q = await app.waitForQuestion(10_000);
    await answer(q.id);
  }
  const until = Date.now() + 15_000;
  while (Date.now() < until) {
    const n = Number(await app.eval("__VN_TEST_MIC__.results.length"));
    if (n > before) return String(await app.eval(`__VN_TEST_MIC__.results[${before}]`));
    await Bun.sleep(25);
  }
  throw new Error("getUserMedia never settled");
}

async function inputs(app: VnApp): Promise<string> {
  await app.eval(
    "globalThis.__ins = null; navigator.mediaDevices.enumerateDevices().then(ds => { globalThis.__ins = " +
      "ds.filter(d => d.kind === 'audioinput').map(d => d.deviceId + '/' + d.label).join(','); }); 0",
  );
  for (let i = 0; i < 100; i++) {
    const v = await app.eval("globalThis.__ins");
    if (v !== null) return String(v);
    await Bun.sleep(20);
  }
  throw new Error("enumerateDevices never settled");
}

async function listed(app: VnApp, kind: string, request = true): Promise<string> {
  const before = Number(await app.eval("__VN_TEST_MIC__.listed.length"));
  await app.eval(`__VN_TEST_MIC__.devices(${JSON.stringify(kind)}, ${request}), 0`);
  for (let i = 0; i < 400; i++) {
    if (Number(await app.eval("__VN_TEST_MIC__.listed.length")) > before) {
      return String(await app.eval(`__VN_TEST_MIC__.listed[${before}]`));
    }
    await Bun.sleep(25);
  }
  throw new Error("getLocalDevices never settled");
}

let app: VnApp;

test(
  "nothing is decided at first: prompt, anonymous inputs, and the question names the app",
  async () => {
    app = await host("fake", "first");
    expect(await state(app)).toBe("prompt");
    expect(await inputs(app)).toBe("/");
    await app.eval("__VN_TEST_MIC__.open(), 0");
    const q = await app.waitForQuestion(10_000);
    expect(q.what).toBe("permission");
    expect(q.title).toBe("Mic Fixture");
    expect(q.instruction).toMatch(/^Mic Fixture (wants to use your microphone|запрашивает доступ к микрофону)$/);
    expect(q.buttons.length).toBe(2);
    await app.cancelDialog(q.id);
    const until = Date.now() + 10_000;
    let got = "";
    while (!got && Date.now() < until) {
      got = String((await app.eval("__VN_TEST_MIC__.results[0] ?? ''")) ?? "");
      await Bun.sleep(25);
    }
    expect(got).toBe("err:NotAllowedError: Permission dismissed");
    expect(await state(app)).toBe("prompt");
    expect((await app.permissions()).decisions).toEqual([]);
  },
  T,
);

test(
  "Allow opens the microphone, is remembered, and later calls do not ask",
  async () => {
    const got = await open(app, (id) => app.chooseDialog(id, 0));
    expect(got).toBe("ok:live:Fake Microphone");
    expect(await state(app)).toBe("granted");
    expect(String(await app.eval("__VN_TEST_MIC__.changes.join(',')"))).toBe("granted");
    const info = await app.permissions();
    expect(info.decisions.map((d) => `${d.name}:${d.decision}`)).toEqual(["microphone:allow"]);
    expect(info.file).toMatch(/permissions\.json$/);
    appDirs.add(dirname(dirname(info.file!)));
    expect(JSON.parse(readFileSync(info.file!, "utf8")).permissions.microphone.decision).toBe("allow");
    expect(await open(app)).toBe("ok:live:Fake Microphone");
    expect(await app.questions()).toEqual([]);
    expect(await inputs(app)).toBe("default/Default - Fake Microphone,fake/Fake Microphone");
    await app.eval("__VN_TEST_MIC__.stopAll(), 0");
  },
  T,
);

test(
  "the inputs are listed by name, and the OS device notifications are on",
  async () => {
    // Granted, so the listing has names and asks nothing.
    expect(await listed(app, "audioinput")).toBe("ok:default/Default - Fake Microphone,fake/Fake Microphone");
    expect(await app.questions()).toEqual([]);
    expect(await listed(app, "videoinput")).toBe("ok:");
    await app.eval("navigator.mediaDevices.addEventListener('devicechange', () => {}), 0");
    let log = "";
    for (let i = 0; i < 100 && !log.includes("audio device notifications"); i++) {
      log = readFileSync(app.hostLog, "utf8");
      await Bun.sleep(50);
    }
    expect(log).toContain("[vn] audio device notifications on");
  },
  T,
);

test(
  "Block refuses, and getUserMedia never asks again until a reset or appPermissions.request",
  async () => {
    const reset = await app.resetPermissions("microphone");
    expect(reset.reset).toEqual(["microphone"]);
    expect(await state(app)).toBe("prompt");
    expect(await open(app, (id) => app.chooseDialog(id, "1"))).toBe("err:NotAllowedError: Permission denied");
    expect(await state(app)).toBe("denied");
    expect(await open(app)).toBe("err:NotAllowedError: Permission denied");
    expect(await app.questions()).toEqual([]);
    await app.eval(
      "globalThis.__req = ''; __VN_TEST_MIC__.appPermissions.request('microphone').then(s => { globalThis.__req = s; }); 0",
    );
    const q = await app.waitForQuestion(10_000);
    await app.chooseDialog(q.id, 0);
    let req = "";
    for (let i = 0; i < 200 && !req; i++) {
      req = String(await app.eval("globalThis.__req"));
      await Bun.sleep(25);
    }
    expect(req).toBe("granted");
    expect(await open(app)).toBe("ok:live:Fake Microphone");
    await app.eval("__VN_TEST_MIC__.stopAll(), 0");
  },
  T,
);

test(
  "the answer outlives the process",
  async () => {
    await app.close();
    app = await host("fake", "second");
    expect(await state(app)).toBe("granted");
    expect(await open(app)).toBe("ok:live:Fake Microphone");
    expect(await app.questions()).toEqual([]);
    await app.eval("__VN_TEST_MIC__.stopAll(), 0");
  },
  T,
);

test(
  "without an embedder's processing, getSettings reports that none runs",
  async () => {
    const opened = async (constraints: string) => {
      const before = Number(await app.eval("__VN_TEST_MIC__.settings.length"));
      await app.eval(`__VN_TEST_MIC__.open(${constraints}), 0`);
      for (let i = 0; i < 400; i++) {
        if (Number(await app.eval("__VN_TEST_MIC__.settings.length")) > before) break;
        await Bun.sleep(25);
      }
      return String(await app.eval(`__VN_TEST_MIC__.settings[${before}]`));
    };
    expect(await opened("{ audio: true }")).toBe("false|false|false|1");
    expect(await opened("{ audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false } }")).toBe(
      "false|false|false|1",
    );
    await app.eval("__VN_TEST_MIC__.stopAll(), 0");
  },
  T,
);

test(
  "VN_MEDIA_CAPTURE=deny refuses without a prompt, whatever was stored",
  async () => {
    await app.close();
    app = await host("deny", "deny");
    expect(await state(app)).toBe("denied");
    expect(await open(app)).toBe("err:NotAllowedError: Permission denied: microphone capture is switched off for this process.");
    expect(await listed(app, "audioinput")).toBe(
      "err:NotAllowedError: Permission denied: microphone capture is switched off for this process.",
    );
    expect(await listed(app, "audioinput", false)).toBe("ok:/");
    expect(await listed(app, "audiooutput")).toMatch(/^ok:default\/[^,]+/);
    await app.eval("__VN_TEST_MIC__.appPermissions.request('microphone'), 0");
    await Bun.sleep(300);
    expect(await app.questions()).toEqual([]);
    const info = await app.permissions();
    expect(info.decisions.map((d) => `${d.name}:${d.decision}`)).toEqual(["microphone:allow"]);
    expect(existsSync(join(dirname(info.file!), "permissions.json"))).toBe(true);
  },
  T,
);
