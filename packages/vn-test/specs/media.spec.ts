// MediaStream and the two MediaStream audio nodes, in a real host (m26 fixes,
// research/reports/m26/client-sweep-1.md finding 11; redjs webapi/mediastream.rs,
// crates/vn-voice/src/track_capture.rs).
//
//   bun test packages/vn-test/specs/media.spec.ts
//
// The harness launches every host with VN_MEDIA_CAPTURE=deny, so nothing here
// can open the machine's microphone: getUserMedia must refuse, by name. What
// IS exercised is the graph: an oscillator into a MediaStreamAudioDestinationNode,
// whose track feeds a MediaStreamAudioSourceNode in the same context, read
// back through an analyser, which is how the Argon client builds its
// microphone path. Any fixture will do; this one borrows `events`.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "media",
});
afterAll(() => app.close());

const T = 60_000;

/** Evaluate an async body in the page; its settled value, JSON-decoded. */
async function inPage(body: string): Promise<unknown> {
  await app.eval(
    `globalThis.__media = undefined; (async () => { ${body} })().then(` +
      `v => { globalThis.__media = JSON.stringify({ ok: v }); }, ` +
      `e => { globalThis.__media = JSON.stringify({ err: (e && e.name) + ': ' + (e && e.message) }); }); 'started'`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await app.eval("globalThis.__media === undefined ? '' : globalThis.__media");
    if (raw) return JSON.parse(String(raw));
    await Bun.sleep(50);
  }
  throw new Error("the page body never settled");
}

test("the classes are there, tagged as a browser tags them", async () => {
  const got = await inPage(
    "return ['MediaStream', 'MediaStreamTrack', 'MediaStreamAudioSourceNode', 'MediaStreamAudioDestinationNode']" +
      ".map(n => Object.prototype.toString.call(globalThis[n].prototype)).join();",
  );
  expect(got).toEqual({
    ok: "[object MediaStream],[object MediaStreamTrack],[object MediaStreamAudioSourceNode],[object MediaStreamAudioDestinationNode]",
  });
  // The nodes themselves too (fixes-host-media-1 §4: every node said
  // [object AudioNode]).
  const nodes = await inPage(`
    const ctx = new AudioContext();
    const dest = ctx.createMediaStreamDestination();
    const tags = [dest, ctx.createMediaStreamSource(dest.stream), ctx.createGain(), ctx.destination]
      .map(v => Object.prototype.toString.call(v)).join();
    await ctx.close();
    return tags;
  `);
  expect(nodes).toEqual({
    ok: "[object MediaStreamAudioDestinationNode],[object MediaStreamAudioSourceNode],[object GainNode],[object AudioDestinationNode]",
  });
}, T);

test("getUserMedia refuses with NotAllowedError, and no input is named", async () => {
  const refused = await inPage("await navigator.mediaDevices.getUserMedia({ audio: true }); return 'opened';");
  expect(String((refused as { err: string }).err)).toContain("NotAllowedError");
  const inputs = await inPage(
    "return (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === 'audioinput')" +
      ".map(d => [d.deviceId, d.label, d instanceof InputDeviceInfo, JSON.stringify(d.getCapabilities())].join('|'));",
  );
  // At most one anonymous entry (a microphone exists, unnamed): never a device
  // the page could offer and then fail to open. It is an InputDeviceInfo with
  // nothing to describe.
  const list = (inputs as { ok: string[] }).ok;
  expect(list.length).toBeLessThanOrEqual(1);
  for (const row of list) expect(row).toBe("||true|{}");
}, T);

test("a destination node's track plays into a source node in the same graph", async () => {
  const got = await inPage(`
    const ctx = globalThis.__mctx = new AudioContext();
    const dest = ctx.createMediaStreamDestination();
    const osc = ctx.createOscillator();
    osc.connect(dest);
    osc.start();
    const track = dest.stream.getAudioTracks()[0];
    const src = ctx.createMediaStreamSource(new MediaStream([track.clone()]));
    const an = ctx.createAnalyser();
    src.connect(an);
    await new Promise(r => setTimeout(r, 600));
    const b = new Float32Array(an.fftSize);
    an.getFloatTimeDomainData(b);
    let e = 0; for (const v of b) e += v * v;
    const rms = Math.sqrt(e / b.length);
    const moved = ctx.currentTime > 0;
    return { kind: track.kind, state: track.readyState, ports: [dest.numberOfInputs, dest.numberOfOutputs, src.numberOfInputs, src.numberOfOutputs], moved, heard: !moved || rms > 0.05, id: __rjs_mediaTrackId(track) > 0 };
  `);
  // Whether a real output device ran the graph (a headless box has none and
  // the signal check is moot), for whoever reads the log.
  console.log(`[media] ${JSON.stringify(got)}`);
  expect(got).toEqual({
    ok: { kind: "audio", state: "live", ports: [1, 0, 0, 1], moved: expect.any(Boolean), heard: true, id: true },
  });
}, T);

// The audio-atlas case (Argon's tone store, @argon/soundfx createAudioAtlas):
// `start(when, offset, duration)` on a buffer holding many sounds must play the
// one slice. Before redjs 0b15d66 `duration` was ignored and the source ran on
// to the end of the buffer, so `onended` came ~0.5 s late here. Muted output.
test("a sprite plays only its slice of the buffer: onended after its duration", async () => {
  const got = await inPage(`
    const ctx = new AudioContext();
    const mute = ctx.createGain(); mute.gain.value = 0; mute.connect(ctx.destination);
    const buf = ctx.createBuffer(1, ctx.sampleRate, ctx.sampleRate); // 1 s
    const ch = buf.getChannelData(0); for (let i = 0; i < ch.length; i++) ch[i] = Math.sin(i / 10) * 0.5;
    buf.copyToChannel(ch, 0);
    const src = ctx.createBufferSource();
    src.buffer = buf; src.connect(mute);
    const ended = new Promise(r => { src.onended = r; });
    const t0 = ctx.currentTime;
    src.start(0, 0.5, 0.05);
    const timeout = new Promise(r => setTimeout(() => r("timeout"), 3000));
    const why = await Promise.race([ended.then(() => "ended"), timeout]);
    const took = ctx.currentTime - t0;
    const moved = ctx.currentTime > 0;
    ctx.close();
    const loop = (() => { const s = new AudioContext().createBufferSource(); s.loopStart = 0.25; s.loopEnd = 0.5; return [s.loopStart, s.loopEnd]; })();
    return { moved, why, inTime: !moved || took < 0.3, loop };
  `);
  console.log(`[media] sprite ${JSON.stringify(got)}`);
  const r = (got as { ok: { moved: boolean; why: string } }).ok;
  // With no output device the clock does not run and nothing ends; the slice
  // itself is pinned by redjs's offline-graph tests.
  if (r.moved) expect(got).toEqual({ ok: { moved: true, why: "ended", inTime: true, loop: [0.25, 0.5] } });
}, T);

// The Argon dialpad (apps/client packages/audio/src/DTMF.ts, verbatim but for
// the output node): per key, two oscillators through two stereo panners, a
// gain whose envelope is scheduled in the same task (0 at `now`, up to 0.075
// in 8 ms, down to 0 at 120 ms) and a low-pass, started at once, and nothing
// kept but the oscillators. Pressed 20 times a second for 3 s, while the spec
// forces a garbage collection every ~60 ms, as a busy client page does.
// Read through an analyser into a muted gain; nothing reaches a speaker.
//
// What the owner heard, and what this measures:
// - a gain still at its default 1 when the oscillators start is a full-scale
//   dual tone (peaks near 2) at every key. Each tone peaks near 0.1 and a
//   few overlap, so the mix never comes near 0.6;
// - a gain applied squared (0.075 as 0.0056) leaves the tones near 0.01, so
//   the mix must also reach 0.05;
// - a gain, panner or filter collected while its oscillators still play
//   leaves the graph, and the tone stops mid-way. The keys overlap, so the
//   mix is never silent while they are pressed: a 10 ms stretch of silence
//   inside the burst is a tone cut short.
test("a fast dialpad: no full-scale burst, and no tone cut short by a collection", async () => {
  await app.eval(`
    globalThis.__dial = undefined;
    (async () => {
      const ctx = new AudioContext();
      const bus = ctx.createGain();
      const meter = ctx.createAnalyser(); meter.fftSize = 4096;
      const mute = ctx.createGain(); mute.gain.value = 0;
      bus.connect(meter); meter.connect(mute); mute.connect(ctx.destination);
      const map = { "1": [697, 1209], "2": [697, 1336], "3": [697, 1477], "4": [770, 1209], "5": [770, 1336],
        "6": [770, 1477], "7": [852, 1209], "8": [852, 1336], "9": [852, 1477], "*": [941, 1209],
        "0": [941, 1336], "#": [941, 1477] };
      const playDTMF = (key) => {
        const freqs = map[key];
        const now = ctx.currentTime;
        const duration = 0.12, attack = 0.008, release = 0.02;
        const oscLow = ctx.createOscillator(), oscHigh = ctx.createOscillator();
        const panLow = ctx.createStereoPanner(), panHigh = ctx.createStereoPanner();
        const gain = ctx.createGain(), filter = ctx.createBiquadFilter();
        oscLow.type = "sine"; oscHigh.type = "sine";
        oscLow.frequency.value = freqs[0]; oscHigh.frequency.value = freqs[1];
        panLow.pan.value = -0.12; panHigh.pan.value = 0.12;
        gain.gain.setValueAtTime(0, now);
        gain.gain.linearRampToValueAtTime(0.075, now + attack);
        gain.gain.setValueAtTime(0.075, now + duration - release);
        gain.gain.linearRampToValueAtTime(0, now + duration);
        filter.type = "lowpass"; filter.frequency.value = 3000; filter.Q.value = 0.7;
        oscLow.connect(panLow).connect(gain);
        oscHigh.connect(panHigh).connect(gain);
        gain.connect(filter);
        filter.connect(bus);
        oscLow.start(now); oscHigh.start(now);
        oscLow.stop(now + duration); oscHigh.stop(now + duration);
      };
      const buf = new Float32Array(4096);
      const presses = 60, every = 50;
      let peak = 0, loud = 0, polls = 0, silent = 0, gap = 0, n = 0;
      let last = performance.now(), first = 0;
      const poll = setInterval(() => {
        const t = performance.now(); gap = Math.max(gap, t - last); last = t;
        meter.getFloatTimeDomainData(buf);
        let p = 0; for (let i = 0; i < buf.length; i++) { const a = Math.abs(buf[i]); if (a > p) p = a; }
        if (p > peak) peak = p;
        if (p > 0.6) loud++;
        // The newest 10 ms: inside the burst, past its first tone and before
        // its last one ends, the mix is never silent.
        const since = t - first;
        if (first && since > 250 && since < (presses - 1) * every) {
          let e = 0; for (let i = buf.length - 480; i < buf.length; i++) e += buf[i] * buf[i];
          if (Math.sqrt(e / 480) < 0.005) silent++;
        }
        polls++;
      }, 10);
      const t0 = ctx.currentTime;
      const keys = "123456789*0#";
      await new Promise(r => { const id = setInterval(() => {
        if (!first) first = performance.now();
        playDTMF(keys[n++ % 12]);
        if (n >= presses) { clearInterval(id); r(); }
      }, every); });
      await new Promise(r => setTimeout(r, 400));
      clearInterval(poll);
      const moved = ctx.currentTime - t0 > 1;
      ctx.close();
      return { moved, presses: n, peak: Math.round(peak * 1000) / 1000, loud, silent, polls, gapMs: Math.round(gap) };
    })().then(v => { globalThis.__dial = JSON.stringify({ ok: v }); },
              e => { globalThis.__dial = JSON.stringify({ err: String(e && e.message) }); });
    'started'`);
  let raw = "";
  let collections = 0;
  const until = Date.now() + 20_000;
  while (!raw && Date.now() < until) {
    await app.send("mem", { gc: true });
    collections++;
    await Bun.sleep(60);
    raw = String((await app.eval("globalThis.__dial ?? ''")) ?? "");
  }
  const got = JSON.parse(raw || "{}");
  console.log(`[media] dialpad ${JSON.stringify(got)}, ${collections} forced collections`);
  const r = (got as { ok: { moved: boolean; presses: number; peak: number; loud: number; silent: number } }).ok;
  expect(r.presses).toBe(60);
  // With no output device the clock does not run and nothing plays; the
  // envelope and the node lifetimes are then pinned by redjs's own tests.
  if (r.moved) {
    // Heard, at the envelope's level and not its square.
    expect(r.peak).toBeGreaterThan(0.05);
    expect(r.peak).toBeLessThan(0.6);
    expect(r.loud).toBe(0);
    expect(r.silent).toBe(0);
  }
}, T);

test("the app logged nothing broken", async () => {
  await app.expectNoErrors();
}, T);
