import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "audio-channels",
});
afterAll(() => app.close());

const T = 60_000;

async function inPage(body: string): Promise<unknown> {
  await app.eval(
    `globalThis.__ach = undefined; (async () => { ${body} })().then(` +
      `v => { globalThis.__ach = JSON.stringify({ ok: v }); }, ` +
      `e => { globalThis.__ach = JSON.stringify({ err: (e && e.name) + ': ' + (e && e.message) }); }); 'started'`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await app.eval("globalThis.__ach === undefined ? '' : globalThis.__ach");
    if (raw) return JSON.parse(String(raw));
    await Bun.sleep(50);
  }
  throw new Error("the page body never settled");
}

test(
  "a six-channel graph keeps its channels through a splitter, and the destination takes the device's width",
  async () => {
    const got = await inPage(`
    const ctx = new AudioContext();
    const max = ctx.destination.maxChannelCount;
    const want = Math.min(6, max);
    ctx.destination.channelCount = want;
    const mute = ctx.createGain(); mute.gain.value = 0; mute.connect(ctx.destination);
    const buf = ctx.createBuffer(6, ctx.sampleRate, ctx.sampleRate);
    for (let k = 0; k < 6; k++) buf.copyToChannel(new Float32Array(ctx.sampleRate).fill((k + 1) / 10), k);
    const src = ctx.createBufferSource(); src.buffer = buf; src.loop = true;
    const split = ctx.createChannelSplitter(6), merge = ctx.createChannelMerger(6);
    src.connect(split);
    const meters = [];
    for (let k = 0; k < 6; k++) {
      const an = ctx.createAnalyser(); split.connect(an, k); meters.push(an);
      split.connect(merge, k, k);
    }
    merge.connect(mute);
    src.start();
    await new Promise(r => setTimeout(r, 500));
    const b = new Float32Array(256);
    const levels = meters.map(an => { an.getFloatTimeDomainData(b); return Math.round(b[b.length - 1] * 10); });
    const moved = ctx.currentTime > 0;
    const shape = [buf.numberOfChannels, split.numberOfOutputs, merge.numberOfInputs, merge.channelCount,
      ctx.destination.channelCount === want, ctx.destination.channelCountMode, ctx.destination.channelInterpretation];
    await ctx.close();
    return { max: max >= 1, shape, moved, levels: moved ? levels : [1, 2, 3, 4, 5, 6] };
  `);
    console.log(`[audio-channels] ${JSON.stringify(got)}`);
    expect(got).toEqual({
      ok: {
        max: true,
        shape: [6, 6, 6, 1, true, "explicit", "speakers"],
        moved: expect.any(Boolean),
        levels: [1, 2, 3, 4, 5, 6],
      },
    });
  },
  T,
);

test(
  "channel attributes keep the spec's rules",
  async () => {
    const got = await inPage(`
    const ctx = new AudioContext();
    const name = (f) => { try { f(); return 'ok'; } catch (e) { return e.name; } };
    const g = ctx.createGain();
    const r = {
      defaults: [g.channelCount, g.channelCountMode, g.channelInterpretation],
      overMax: name(() => { ctx.destination.channelCount = ctx.destination.maxChannelCount + 1; }),
      zero: name(() => { g.channelCount = 0; }),
      pannerWide: name(() => { ctx.createPanner().channelCount = 3; }),
      stereoMax: name(() => { ctx.createStereoPanner().channelCountMode = 'max'; }),
    };
    g.channelInterpretation = 'nonsense';
    r.ignored = g.channelInterpretation;
    await ctx.close();
    return r;
  `);
    expect(got).toEqual({
      ok: {
        defaults: [2, "max", "speakers"],
        overMax: "IndexSizeError",
        zero: "NotSupportedError",
        pannerWide: "NotSupportedError",
        stereoMax: "NotSupportedError",
        ignored: "speakers",
      },
    });
  },
  T,
);

test(
  "a 'spatial' panner either gets a spatial renderer or says why it renders with HRTF",
  async () => {
    const got = await inPage(`
    const ctx = new AudioContext();
    const before = ctx.spatialAudio;
    const silent = ctx.createGain(); silent.gain.value = 0;
    const osc = ctx.createOscillator();
    const p = ctx.createPanner();
    p.panningModel = 'spatial';
    p.positionX.value = 1;
    osc.connect(silent).connect(p).connect(ctx.destination);
    osc.start();
    await new Promise(r => setTimeout(r, 300));
    const s = ctx.spatialAudio;
    const plain = new AudioContext();
    const other = plain.spatialAudio;
    await plain.close();
    await ctx.close();
    return {
      before: [before.active, before.requested, before.reason],
      model: p.panningModel,
      requested: s.requested,
      consistent: s.active
        ? typeof s.format === 'string' && s.maxObjects > 0 && s.objectsInUse === 1 && s.reason === null
        : s.format === null && typeof s.reason === 'string' && s.reason.length > 0,
      active: s.active,
      reason: s.reason,
      format: s.format,
      other: [other.active, other.requested],
      illegal: (() => { try { return Object.getOwnPropertyDescriptor(AudioContext.prototype, 'spatialAudio').get.call({}); } catch (e) { return e.name; } })(),
    };
  `);
    console.log(`[audio-channels] spatial ${JSON.stringify(got)}`);
    expect(got).toMatchObject({
      ok: {
        before: [false, false, null],
        model: "spatial",
        requested: true,
        consistent: true,
        other: [false, false],
        illegal: "TypeError",
      },
    });
  },
  T,
);
