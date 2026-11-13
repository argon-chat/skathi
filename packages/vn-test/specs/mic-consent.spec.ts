// getUserMedia behind the app's own permission prompt, in a real host (m26
// voice-fixes-1 item 1; docs/PERMISSIONS.md §5.1, crates/vn-permissions
// consent.rs, redjs webapi/mediastream.rs "The capture gate").
//
//   bun test packages/vn-test/specs/mic-consent.spec.ts
//
// NOTHING HERE OPENS A MICROPHONE. The hosts run with VN_MEDIA_CAPTURE=fake
// (a synthetic 440 Hz tone; the only other value vn-test lets through is
// `deny`) and VN_DIALOGS=scripted, so the prompt is a question the spec
// answers with `chooseDialog` / `cancelDialog` and never reaches the screen.
// Each host has a VN_APP_NAME of its own, so the stored answer
// (`permissions.json` in its storage folder) is this run's, and is deleted at
// the end.

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
    // The microphone module at info: the device notifier says it registered.
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

/** getUserMedia, then the result it settles with (after `answer`, if given). */
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

/** The fixture's `devices(kind, request)` (a call SDK's device listing), settled. */
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
    // Esc: dismissed, Chrome's words, nothing stored.
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
    // Granted: no question, and the inputs have names.
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
    // devicechange comes from IMMNotificationClient, not a poll
    // (crates/vn-app/src/microphone/devices.rs). A listener arms it.
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
    // The extension's request is the app's way back.
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
    // Echo cancellation and the rest are the embedder's
    // (`mediastream::set_capture_processing`); the engine has none, and says so
    // whatever the constraints ask for.
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
    // The listing asks getUserMedia for the names, and the refusal is its
    // result. Without the request, the nameless entry.
    expect(await listed(app, "audioinput")).toBe(
      "err:NotAllowedError: Permission denied: microphone capture is switched off for this process.",
    );
    expect(await listed(app, "audioinput", false)).toBe("ok:/");
    expect(await listed(app, "audiooutput")).toMatch(/^ok:default\/[^,]+/);
    await app.eval("__VN_TEST_MIC__.appPermissions.request('microphone'), 0");
    await Bun.sleep(300);
    expect(await app.questions()).toEqual([]);
    // The stored answer is untouched.
    const info = await app.permissions();
    expect(info.decisions.map((d) => `${d.name}:${d.decision}`)).toEqual(["microphone:allow"]);
    expect(existsSync(join(dirname(info.file!), "permissions.json"))).toBe(true);
  },
  T,
);
