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

test("the app logged nothing broken", async () => {
  await app.expectNoErrors();
}, T);
