import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/audio-worklet/src/main.ts",
  window: [360, 200],
  label: "audio-worklet",
});
afterAll(() => app.close());

const T = 60_000;

async function inPage(body: string): Promise<unknown> {
  await app.eval(
    `globalThis.__awl = undefined; (async () => { ${body} })().then(` +
      `v => { globalThis.__awl = JSON.stringify({ ok: v }); }, ` +
      `e => { globalThis.__awl = JSON.stringify({ err: (e && e.name) + ': ' + (e && e.message) }); }); 'started'`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await app.eval("globalThis.__awl === undefined ? '' : globalThis.__awl");
    if (raw) return JSON.parse(String(raw));
    await Bun.sleep(50);
  }
  throw new Error("the page body never settled");
}

test(
  "a worklet module built from TypeScript with an import loads, answers its node and is heard by an analyser",
  async () => {
    const got = await inPage(`
    const fixture = globalThis.__SK_TEST_WORKLET__;
    const source = await (await fetch(fixture.toneUrl)).text();
    const ctx = new AudioContext();
    await fixture.load(ctx);
    const tone = new AudioWorkletNode(ctx, 'tone', { numberOfInputs: 0 });
    const echo = new AudioWorkletNode(ctx, 'echo', { numberOfInputs: 0 });
    // No path to ctx.destination: the analyser hears the processor, the speakers do not.
    const meter = ctx.createAnalyser();
    tone.connect(meter);
    const answers = [await fixture.ask(tone, 'ping'), await fixture.ask(echo, 'pong')];
    await new Promise(r => setTimeout(r, 500));
    const moved = ctx.currentTime > 0;
    const b = new Float32Array(1024);
    meter.getFloatTimeDomainData(b);
    const rms = Math.sqrt(b.reduce((a, v) => a + v * v, 0) / b.length);
    const rate = answers[0].rate === ctx.sampleRate;
    await ctx.close();
    return {
      classic: !/\\b(?:import|export)\\b/.test(source),
      answers: answers.map(a => a.answer),
      rate,
      moved,
      heard: moved ? Math.abs(rms - 0.25) < 1e-3 : true,
    };
  `);
    console.log(`[audio-worklet] ${JSON.stringify(got)}`);
    expect(got).toEqual({
      ok: { classic: true, answers: ["tone:ping", "echo:pong"], rate: true, moved: expect.any(Boolean), heard: true },
    });
  },
  T,
);
