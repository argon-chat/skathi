import { expect, test } from "bun:test";
import { installMediaSession } from "../src/mediasession";

interface Session {
  metadata: unknown;
  playbackState: string;
  setActionHandler(action: string, handler: unknown): void;
  setPositionState(state?: Record<string, number>): void;
}
type MetadataCtor = new (init?: Record<string, unknown>) => { title: string; artwork: readonly { src: string }[] };

function setup() {
  const updates: { state: Record<string, unknown>; art: ArrayBuffer | undefined }[] = [];
  let act: (p: Record<string, unknown>) => void = () => {};
  const errors: unknown[] = [];
  const nav = {} as { mediaSession: Session };
  installMediaSession(nav, {
    host: { update: (json, art) => void updates.push({ state: JSON.parse(json), art }) },
    resolveUrl: (src) => new URL(src, "app://redjs/index.html").href,
    fetchBytes: () => Promise.resolve(new Uint8Array([1, 2, 3]).buffer),
    onAction: (fn) => void (act = fn),
    reportError: (e) => void errors.push(e),
  });
  const MediaMetadata = (globalThis as unknown as { MediaMetadata: MetadataCtor }).MediaMetadata;
  return { s: nav.mediaSession, MediaMetadata, updates, act: (p: Record<string, unknown>) => act(p), errors };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("one update per task, with metadata, state, handlers and position", async () => {
  const { s, MediaMetadata, updates } = setup();
  s.metadata = new MediaMetadata({
    title: "Song",
    artist: "Band",
    artwork: [
      { src: "https://x.test/s.png", sizes: "96x96" },
      { src: "https://x.test/l.png", sizes: "512x512" },
    ],
  });
  s.playbackState = "playing";
  s.playbackState = "bogus";
  s.setActionHandler("play", () => {});
  s.setActionHandler("nexttrack", () => {});
  s.setActionHandler("play", null);
  s.setPositionState({ duration: 100, position: 5 });
  await tick();
  expect(updates.length).toBe(1);
  expect(updates[0]!.state).toEqual({
    metadata: { title: "Song", artist: "Band", album: "", artworkUrl: "https://x.test/l.png" },
    playbackState: "playing",
    actions: ["nexttrack"],
    position: { duration: 100, position: 5, playbackRate: 1 },
  });
  (s.metadata as { title: string }).title = "Next";
  await tick();
  expect((updates[1]!.state.metadata as { title: string }).title).toBe("Next");
});

test("local artwork is sent as bytes", async () => {
  const { s, MediaMetadata, updates } = setup();
  s.metadata = new MediaMetadata({ title: "T", artwork: [{ src: "cover.png" }] });
  expect((s.metadata as { artwork: { src: string }[] }).artwork[0]!.src).toBe("app://redjs/cover.png");
  await tick();
  await tick();
  expect(updates.map((u) => u.art?.byteLength)).toEqual([undefined, 3]);
});

test("host buttons call the handler with its details", async () => {
  const { s, act, errors } = setup();
  const seen: unknown[] = [];
  s.setActionHandler("seekto", (d: unknown) => seen.push(d));
  s.setActionHandler("pause", () => {
    throw new Error("boom");
  });
  act({ kind: "mediasession", action: "seekto", seekTime: 42 });
  act({ kind: "mediasession", action: "play" });
  act({ kind: "mediasession", action: "pause" });
  expect(seen).toEqual([{ action: "seekto", seekTime: 42 }]);
  expect(errors.length).toBe(1);
});

test("what the spec refuses is a TypeError", () => {
  const { s, MediaMetadata } = setup();
  expect(() => s.setActionHandler("fly", null)).toThrow(TypeError);
  expect(() => s.setActionHandler("play", 1)).toThrow(TypeError);
  expect(() => (s.metadata = {})).toThrow(TypeError);
  expect(() => s.setPositionState({ position: 1 })).toThrow(TypeError);
  expect(() => s.setPositionState({ duration: 10, position: 11 })).toThrow(TypeError);
  expect(() => s.setPositionState({ duration: 10, playbackRate: 0 })).toThrow(TypeError);
  expect(() => s.setPositionState({ duration: -1 })).toThrow(TypeError);
  expect(() => new MediaMetadata({ artwork: [{}] })).toThrow(TypeError);
  expect(() => new MediaMetadata({ artwork: [{ src: "http://[" }] })).toThrow(TypeError);
  s.setPositionState({ duration: Infinity });
  s.setPositionState();
});
