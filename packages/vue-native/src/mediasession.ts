// Media Session: `MediaMetadata` and `navigator.mediaSession`, handed to the system's media controls.
// Changes in one task reach the host as one update; artwork that is not http(s) is read here and sent
// as bytes.

export interface MediaSessionHost {
  update(stateJson: string, artwork: ArrayBuffer | undefined): void;
}

export interface MediaSessionDeps {
  host: MediaSessionHost;
  /** Throws on an invalid URL. */
  resolveUrl(src: string): string;
  fetchBytes(url: string): Promise<ArrayBuffer>;
  onAction(fn: (payload: Record<string, unknown>) => void): void;
  reportError(e: unknown): void;
}

export interface MediaImage {
  src: string;
  sizes?: string;
  type?: string;
}

interface MediaMetadataInit {
  title?: string;
  artist?: string;
  album?: string;
  artwork?: readonly MediaImage[];
}

type Handler = (details: { action: string; seekTime?: number }) => unknown;

const ACTIONS = [
  "play",
  "pause",
  "seekbackward",
  "seekforward",
  "previoustrack",
  "nexttrack",
  "skipad",
  "stop",
  "seekto",
  "togglemicrophone",
  "togglecamera",
  "togglescreenshare",
  "hangup",
  "previousslide",
  "nextslide",
  "enterpictureinpicture",
  "voiceactivity",
];
const STATES = ["none", "paused", "playing"];

function enumError(method: string, value: unknown, type: string): TypeError {
  return new TypeError(
    `Failed to execute '${method}' on 'MediaSession': The provided value '${String(value)}' is not a valid enum value of type ${type}.`,
  );
}

function area(sizes: string): number {
  let best = 0;
  for (const s of sizes.toLowerCase().split(/\s+/)) {
    if (s === "any") return Infinity;
    const m = /^(\d+)x(\d+)$/.exec(s);
    if (m) best = Math.max(best, Number(m[1]) * Number(m[2]));
  }
  return best;
}

export function installMediaSession(nav: object, deps: MediaSessionDeps): void {
  const { host } = deps;
  let changed: () => void = () => {};

  const images = (list: unknown): readonly MediaImage[] => {
    if (list === undefined) return Object.freeze([]);
    if (list === null || typeof list !== "object" || !(Symbol.iterator in list)) {
      throw new TypeError(
        "Failed to read the 'artwork' property from 'MediaMetadataInit': The provided value cannot be converted to a sequence.",
      );
    }
    return Object.freeze(
      [...(list as Iterable<MediaImage>)].map((img) => {
        if (!img || img.src === undefined) {
          throw new TypeError("Failed to read the 'src' property from 'MediaImage': Required member is undefined.");
        }
        let src: string;
        try {
          src = deps.resolveUrl(String(img.src));
        } catch {
          throw new TypeError(
            `Failed to construct 'MediaMetadata': '${String(img.src)}' can't be resolved to a valid URL.`,
          );
        }
        return Object.freeze({ src, sizes: String(img.sizes ?? ""), type: String(img.type ?? "") });
      }),
    );
  };

  class MediaMetadata {
    #title: string;
    #artist: string;
    #album: string;
    #artwork: readonly MediaImage[];
    constructor(init: MediaMetadataInit = {}) {
      this.#title = String(init.title ?? "");
      this.#artist = String(init.artist ?? "");
      this.#album = String(init.album ?? "");
      this.#artwork = images(init.artwork);
    }
    get title(): string {
      return this.#title;
    }
    set title(v: string) {
      this.#title = String(v);
      if (current === this) changed();
    }
    get artist(): string {
      return this.#artist;
    }
    set artist(v: string) {
      this.#artist = String(v);
      if (current === this) changed();
    }
    get album(): string {
      return this.#album;
    }
    set album(v: string) {
      this.#album = String(v);
      if (current === this) changed();
    }
    get artwork(): readonly MediaImage[] {
      return this.#artwork;
    }
    set artwork(v: readonly MediaImage[]) {
      this.#artwork = images(v);
      if (current === this) changed();
    }
  }

  let current: MediaMetadata | null = null;
  let playback = "none";
  let position: { duration: number; position: number; playbackRate: number } | null = null;
  const handlers = new Map<string, Handler>();
  const INTERNAL = Symbol("vn.mediaSession");

  let queued = false;
  let generation = 0;
  let sentArtwork = "";
  let artworkBytes: ArrayBuffer | undefined;
  const send = (): void => {
    queued = false;
    const gen = ++generation;
    const art = current?.artwork.reduce<MediaImage | undefined>(
      (best, img) => (!best || area(img.sizes ?? "") > area(best.sizes ?? "") ? img : best),
      undefined,
    );
    const web = art !== undefined && /^https?:/i.test(art.src);
    const metadata = current && {
      title: current.title,
      artist: current.artist,
      album: current.album,
      ...(web ? { artworkUrl: art.src } : {}),
    };
    const json = JSON.stringify({ metadata, playbackState: playback, actions: [...handlers.keys()], position });
    if (!art || web) {
      sentArtwork = "";
      artworkBytes = undefined;
      host.update(json, undefined);
      return;
    }
    if (art.src === sentArtwork && artworkBytes) {
      host.update(json, artworkBytes);
      return;
    }
    host.update(json, undefined);
    deps.fetchBytes(art.src).then(
      (bytes) => {
        if (gen !== generation) return;
        sentArtwork = art.src;
        artworkBytes = bytes;
        host.update(json, bytes);
      },
      () => {},
    );
  };
  changed = () => {
    if (queued) return;
    queued = true;
    queueMicrotask(send);
  };

  class MediaSession {
    constructor(key?: symbol) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
    }
    get metadata(): MediaMetadata | null {
      return current;
    }
    set metadata(v: MediaMetadata | null) {
      if (v !== null && !(v instanceof MediaMetadata)) {
        throw new TypeError(
          "Failed to set the 'metadata' property on 'MediaSession': The provided value is not of type 'MediaMetadata'.",
        );
      }
      current = v;
      changed();
    }
    get playbackState(): string {
      return playback;
    }
    set playbackState(v: string) {
      if (!STATES.includes(v)) return;
      playback = v;
      changed();
    }
    setActionHandler(action: string, handler: Handler | null): void {
      if (!ACTIONS.includes(action)) throw enumError("setActionHandler", action, "MediaSessionAction");
      if (handler !== null && typeof handler !== "function") {
        throw new TypeError(
          "Failed to execute 'setActionHandler' on 'MediaSession': The callback provided as parameter 2 is not a function.",
        );
      }
      if (handler) handlers.set(action, handler);
      else handlers.delete(action);
      changed();
    }
    setPositionState(state?: { duration?: number; position?: number; playbackRate?: number }): void {
      const fail = (m: string): never => {
        throw new TypeError(`Failed to execute 'setPositionState' on 'MediaSession': ${m}`);
      };
      if (
        !state ||
        (state.duration === undefined && state.position === undefined && state.playbackRate === undefined)
      ) {
        position = null;
        changed();
        return;
      }
      if (state.duration === undefined) fail("The duration must be provided.");
      const duration = Number(state.duration);
      if (Number.isNaN(duration) || duration < 0) fail("The provided duration cannot be less than zero.");
      const at = state.position === undefined ? 0 : Number(state.position);
      if (!Number.isFinite(at)) fail("The provided double value is non-finite.");
      if (at < 0) fail("The provided position cannot be less than zero.");
      if (at > duration) fail("The provided position cannot be greater than the duration.");
      const rate = state.playbackRate === undefined ? 1 : Number(state.playbackRate);
      if (!Number.isFinite(rate)) fail("The provided double value is non-finite.");
      if (rate === 0) fail("The provided playbackRate cannot be equal to zero.");
      position = { duration, position: at, playbackRate: rate };
      changed();
    }
  }

  deps.onAction((payload) => {
    const action = String(payload.action);
    const handler = handlers.get(action);
    if (!handler) return;
    const details: { action: string; seekTime?: number } = { action };
    if (typeof payload.seekTime === "number") details.seekTime = payload.seekTime;
    try {
      handler.call(session, details);
    } catch (e) {
      deps.reportError(e);
    }
  });

  const session = new MediaSession(INTERNAL);
  const g = globalThis as Record<string, unknown>;
  g.MediaMetadata = MediaMetadata;
  g.MediaSession = MediaSession;
  Object.defineProperty(nav, "mediaSession", { value: session, configurable: true, enumerable: true });
}
