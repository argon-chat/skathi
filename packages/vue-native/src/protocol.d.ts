// The `__vn_*` host protocol, typed once.
//
// Every function below is registered on the JS global by a Rust `install()`
// (the file is named on each block), and every one of them is the ONLY way the
// JS runtime can reach the retained tree, the cascade, the media pipeline, the
// permission backend, the storage area or the voice engine. Until this file
// existed the whole seam was untyped: a call with the arguments in the wrong
// order, or a `number` where the host reads a string, produced a silently
// wrong-but-plausible result rather than a build error.
//
// The rules the seam obeys, which the types encode:
//
//   * IDS ARE DENSE u32, and `0` is the reserved "none" id (never a real node).
//     Everything crossing the boundary as an id is a `VnNodeId` / `VnNameId`.
//   * NAMES CROSS ONCE. Tags, style keys and prop keys are interned through
//     `__vn_intern` and travel as `VnNameId` afterwards, so the per-op hot path
//     never carries a string. A host fn that takes a `VnNameId` will silently
//     read garbage if handed a raw string, which is precisely what the types
//     now prevent.
//   * STRUCTURED ANSWERS ARE STRINGS, in one of three shapes: comma-joined
//     numbers (`__vn_query`, `__vn_bounds`), `name=value` lines
//     (`__vn_computed_style`, `__vn_media_state`), or JSON
//     (`__vn_permissions_*`, `__vn_storage_*`, `__vn_voice_*`). The alias types
//     below say which, so a caller cannot `JSON.parse` a line-shaped answer.
//   * A HOST FN MAY BE ABSENT. `vn-storage`, `vn-permissions` and `vn-voice`
//     are installed by the app host, not by the engine, and a build without
//     them (or the unit-test harness) simply has no such global. The runtime
//     probes with `typeof __vn_storage_info === "function"` and falls back to
//     an inert surface — so these are declared as ordinary globals and the
//     probe is what keeps that honest.
//
// Ambient by design: these are engine globals, not module exports, and the
// files that call them are evaluated as CLASSIC SCRIPTS inside the vendor IIFE.

/** A node id in `crates/vn-tree`'s arena. `0` is the reserved "none" id. */
type VnNodeId = number;
/** An interned name id (`__vn_intern`). Dense, stable for the process. */
type VnNameId = number;
/** `"12,40,3"` — matched node ids, or `""` for none. See `__vn_query`. */
type VnIdList = string;
/** `"x,y,w,h"` as f32s, or `""` when the node was not painted. */
type VnRect = string;
/** `name=value` lines, `\n`-separated. `""` when there is nothing to report. */
type VnKvLines = string;
/** A JSON document. The host never returns a bare value — always an object. */
type VnJson = string;

// ---------------------------------------------------------------------------
// the retained tree — crates/vn-tree/src/lib.rs::install
// ---------------------------------------------------------------------------

/**
 * Interns `name`, returning its dense id. Idempotent: the same string always
 * answers the same id, which is what lets the JS side memoise it forever.
 */
declare function __vn_intern(name: string): VnNameId;

/** Creates an element node with the interned `tag`. Returns its id. */
declare function __vn_create_element(tag: VnNameId): VnNodeId;
/** Creates a text node (`kind === 2`). Returns its id. */
declare function __vn_create_text(): VnNodeId;
/** Creates a comment/anchor node (`kind === 3`). Returns its id. */
declare function __vn_create_anchor(): VnNodeId;
/**
 * Creates THE root node. Called exactly once, by `createApp().mount()`.
 * `vn_css::element` answers `:root`, `html` and `body` off this one node.
 */
declare function __vn_create_root(): VnNodeId;

/** Sets a text node's content. */
declare function __vn_set_text(id: VnNodeId, text: string): void;
/** Sets an ELEMENT's own text — exclusive with its children, as in the tree. */
declare function __vn_set_element_text(id: VnNodeId, text: string): void;

/**
 * Inserts `child` into `parent` before `anchor` (`0` = append). Also MOVE: the
 * host detaches `child` from wherever it was first.
 */
declare function __vn_insert(child: VnNodeId, parent: VnNodeId, anchor: VnNodeId): void;
/** Destroys `id` AND ITS WHOLE SUBTREE. The ids are recycled afterwards. */
declare function __vn_remove(id: VnNodeId): void;

/** One inline declaration. `""` REMOVES the property (it does not set empty). */
declare function __vn_set_style(id: VnNodeId, key: VnNameId, value: string): void;
/** The element's whole class attribute, space-joined. Not additive. */
declare function __vn_set_class(id: VnNodeId, classes: string): void;
/**
 * One attribute-channel prop. `undefined` (or `null`) REMOVES it — every other
 * value must already be a string, because the host reads it as one.
 */
declare function __vn_set_prop(id: VnNodeId, key: VnNameId, value: string | undefined): void;
/** Adds a `<style scoped>` data-v id to an element. Additive, never replaces. */
declare function __vn_add_scope(id: VnNodeId, scope: string): void;
/** Replaces the whole stylesheet and marks every node for a fresh cascade. */
declare function __vn_set_stylesheet(css: string): void;

/**
 * Arms or disarms one event bit on one node (`Node::listeners` is a u64, so
 * `code` must stay below 64). The host only delivers an event whose bit is
 * armed, and several JS event NAMES can share one code — see `syncListenerFlag`
 * in `runtime.ts`, which ORs over all of them before calling this.
 */
declare function __vn_set_listener(id: VnNodeId, code: VnEventCode, on: boolean): void;

/**
 * Installs the one dispatcher the host calls for EVERY input event. Returning
 * `false` means a JS handler called `preventDefault()`, which the host honours
 * (`vn_tree::dispatch_event`). The host drains microtasks immediately after the
 * call, so Vue's patch flush lands inside the same frame's commit.
 */
declare function __vn_set_dispatcher(fn: VnDispatcher): void;

/**
 * `(nodeId, code, a, b, c, d, key) -> proceed`.
 *
 * The four numbers and the optional string are ONE payload shape reused for
 * every event; `VnEventPayload` below documents what each slot means per
 * event, mirroring `vn_tree::EventPayload`'s own doc comment.
 */
type VnDispatcher = (
  node: VnNodeId,
  code: VnEventCode,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
) => boolean;

/**
 * What `a`/`b`/`c`/`d`/`key` carry, per event family. Documentation with a
 * compiler behind it: `synthesize()` in `runtime.ts` destructures exactly these.
 *
 * | family                    | a            | b           | c           | d          | key             |
 * |---------------------------|--------------|-------------|-------------|------------|-----------------|
 * | mouse (click/down/up/…)   | clientX      | clientY     | button      | mods       | —               |
 * | wheel                     | deltaX       | deltaY      | —           | mods       | —               |
 * | keydown/keyup             | —            | —           | —           | mods       | `KeyboardEvent.key` |
 * | input                     | —            | —           | —           | —          | the new value   |
 * | scroll                    | scrollLeft   | scrollTop   | —           | —          | —               |
 * | media (crates/vn-video)    | currentTime  | duration    | readyState  | playbackRate | —             |
 * | media `error`             | MediaError.code | currentTime | —        | —          | advisory text   |
 * | lottie `enterFrame`       | currentTime  | totalTime   | elapsed     | progress   | —               |
 * | lottie `loopComplete`     | currentTime  | totalTime   | currentLoop | —          | —               |
 * | lottie `complete`         | currentTime  | totalTime   | loops       | —          | —               |
 * | transitionend             | elapsedTime  | —           | —           | —          | property name   |
 * | animation{start,end,cancel} | elapsedTime | —          | —           | —          | @keyframes name |
 *
 * `mods` is a bitfield: 1 ctrl, 2 shift, 4 alt, 8 meta.
 */
interface VnEventPayload {
  a: number;
  b: number;
  c: number;
  d: number;
  key: string | undefined;
}

/**
 * Every event code the host can deliver, in lockstep with
 * `crates/vn-tree/src/tree.rs` (`pub const EV_*` / `EVENT_NAMES`).
 *
 * A TypeScript `const enum` was the obvious spelling and is deliberately NOT
 * used: bun does not inline one, it DOWNGRADES it to a runtime enum object, so
 * the codes would stop being free and `vendor.js` would grow a table it does
 * not need — and `isolatedModules` forbids an ambient one outright. The numbers
 * therefore stay the `EV_CODES` object literal in `runtime.ts`, which is
 * `satisfies`-checked against this type, and `test/protocol-codes.test.ts`
 * parses `tree.rs` and asserts the two tables are identical. That test is the
 * lockstep guarantee a `const enum` could never have given: it reads the Rust.
 */
type VnEventCode =
  | 1 // click
  | 2 // mousedown
  | 3 // mouseup
  | 4 // wheel
  | 5 // keydown
  | 6 // keyup
  | 7 // focus
  | 8 // blur
  | 9 // input
  | 10 // scroll
  | 11 // complete            (lottie)
  | 12 // loopComplete        (lottie)
  | 13 // enterFrame          (lottie)
  | 14 // mousemove
  | 15 // mouseenter
  | 16 // mouseleave
  | 17 // loadstart           (media)
  | 18 // loadedmetadata
  | 19 // loadeddata
  | 20 // canplay
  | 21 // canplaythrough
  | 22 // play
  | 23 // playing
  | 24 // pause
  | 25 // seeking
  | 26 // seeked
  | 27 // timeupdate
  | 28 // durationchange
  | 29 // ratechange
  | 30 // volumechange
  | 31 // waiting
  | 32 // ended
  | 33 // error
  | 34 // emptied
  | 35 // transitionend       (vn-css animator)
  | 36 // animationstart
  | 37 // animationend
  | 38; // animationcancel

// ---------------------------------------------------------------------------
// the DOM surface — crates/vn-tree/src/dom.rs
// ---------------------------------------------------------------------------

/** `__vn_query`'s mode: 0 descendants, 1 self only (`matches`), 2 `closest`. */
type VnQueryMode = 0 | 1 | 2;

/**
 * Selector matching through the CASCADE'S OWN matcher — never a second JS-side
 * one, which would be a silent divergence from what actually styles the node.
 *
 * Answers the matched ids comma-joined, `""` for no match, or `"!<advisory>"`
 * when the selector is one the engine cannot run (`:has()`, `::before`). The
 * `!` prefix is why the return type is a plain string rather than a list.
 */
declare function __vn_query(selector: string, root: VnNodeId, mode: VnQueryMode): VnIdList;

/**
 * The rectangle gpui laid the node out at during the LAST PAINTED FRAME, in the
 * window's client-area space. `""` for a node that was not painted (detached,
 * `display: none`, parked in a `<KeepAlive>` cache) — which is exactly the
 * all-zero rect a browser reports for those, so it needs no advisory.
 */
declare function __vn_bounds(id: VnNodeId): VnRect;

/** The folded computed values + every custom property, as `name=value` lines. */
declare function __vn_computed_style(id: VnNodeId): VnKvLines;

// ---------------------------------------------------------------------------
// `<video>` / `<audio>` — crates/vn-video (CONTRACT.md §6)
// ---------------------------------------------------------------------------

/** Live media state as `name=value` lines; `""` for a node that is not one. */
declare function __vn_media_state(id: VnNodeId): VnKvLines;

/** `HTMLMediaElement`'s verbs, by code. Mirrors `vn_tree::video::media_call`. */
type VnMediaVerb =
  | 0 // play
  | 1 // pause
  | 2 // currentTime =
  | 3 // volume =
  | 4 // muted =        (arg: 0 | 1)
  | 5 // playbackRate =
  | 6; // loop =        (arg: 0 | 1)

/** Runs one verb. Returns whether the node was a live media element at all. */
declare function __vn_media_call(id: VnNodeId, verb: VnMediaVerb, arg: number): boolean;

/** `canPlayType(type)` — `"" | "maybe" | "probably"`, probed on THIS machine. */
declare function __vn_media_can_play(id: VnNodeId, mime: string): "" | "maybe" | "probably";

/**
 * Drains the node's recent media events, comma-joined. A bridge and a TEST
 * surface — the events themselves ride the ordinary EV_* channel, so nothing
 * in `runtime.ts` calls this; `crates/vn-video`'s own tests do.
 */
declare function __vn_media_events(id: VnNodeId): string;

// ---------------------------------------------------------------------------
// multi-window — crates/vn-app/src/popout.rs (docs/MULTIWINDOW.md)
// ---------------------------------------------------------------------------
//
// PHASE 0, and deliberately raw. These four move a subtree's PAINT into a
// second OS window while the node stays in this tree, this component instance
// and this reactive graph — so `nodeId` is a `VnNodeId`, i.e. `mirrorNode.id`
// off a template ref, and nothing here needs a runtime change (MULTIWINDOW.md
// §5.2 invariant 1). The app-facing spelling is `<VnWindow>` in phase 1 (§5.3);
// until it exists, `apps/popout` calls these directly and this is where their
// shapes are written down.
//
// Installed by `vn-app`, not by the engine — same category as `__vn_storage_*`,
// so a host without them simply has no such global and a caller that cannot
// assume vn-app must probe with `typeof __vn_window_open === "function"`.

/**
 * Moves the node's PAINT into a new OS window.
 *
 * `options` is JSON: `{ title?, x?, y?, width?, height? }`. `onClose` fires
 * with the window, however it closed — the X button, `__vn_window_close`, or
 * the primary window going away.
 *
 * Returns `undefined`, and the open is DEFERRED by one effect cycle:
 * `App::open_window` draws the new window once before returning and this is
 * called from inside the primary window's update, so it cannot be nested there
 * (MULTIWINDOW.md §5.2 invariant 2). UN-painting, though, is immediate — the
 * primary window's next draw is already without the subtree, because a node in
 * two element trees for one frame aliases scroll state and hitboxes.
 */
declare function __vn_window_open(
  nodeId: VnNodeId,
  options: VnJson,
  onClose?: () => void,
): void;

/** Closes the window painting `nodeId` — the same path as the user clicking X. */
declare function __vn_window_close(nodeId: VnNodeId): void;

/** `[{ node, windowId }]` as JSON. For tests and devtools. */
declare function __vn_window_popouts(): VnJson;

/**
 * TEST SURFACE. Dispatches a real platform click into the window painting
 * `nodeId`, at that window's own client coordinates; `true` if it landed.
 */
declare function __vn_window_click(nodeId: VnNodeId, x: number, y: number): boolean;

// ---------------------------------------------------------------------------
// host logging — crates/vn-app/src/main.rs
// ---------------------------------------------------------------------------

/**
 * A line into the host log.
 *
 * `vn-app` registers it UNCONDITIONALLY (`main.rs`, next to `__vn_set_status`
 * and `__vn_perf_mark`) and `runtime.ts` calls it unguarded, which is why this
 * is a `function` rather than an optional `var` — an app does not have to
 * probe. An embedder that installed `vn-tree` without `vn-app` would have to
 * provide it; the unit-test harness does, by binding it as a parameter.
 */
declare function __vn_log(message: string): void;

// ---------------------------------------------------------------------------
// devtools discovery — crates/vn-app/src/devtools/mod.rs (docs/DEVTOOLS-UI.md)
// ---------------------------------------------------------------------------

/**
 * Every attachable vue-native instance, as JSON:
 * `{ preferred, self: { pid, dir }, targets: [{ slot, dir, port, pid, token,
 * alive, self }] }`.
 *
 * The inspector app's ONE host dependency. Everything else it does goes over
 * the ordinary devtools wire, which redjs's `fetch` can speak by itself — but
 * discovery has to read a handshake file off disk, and `fetch` cannot.
 *
 * Registered by `devtools::install`, i.e. in every DEV host and in NO
 * `--features prod` one, so a caller must probe with
 * `typeof __vn_devtools_targets === "function"` and say what the absence means.
 * `apps/devtools/src/wire.ts` is the one caller.
 */
declare function __vn_devtools_targets(): VnJson;

// ---------------------------------------------------------------------------
// permissions — crates/vn-permissions/src/lib.rs (docs/PERMISSIONS.md tier 2)
// ---------------------------------------------------------------------------

/** The four states. `"unsupported"` is the one the web has no word for. */
type VnPermissionState = "granted" | "denied" | "prompt" | "unsupported";
/** The W3C names `Capability::ALL` knows. */
type VnPermissionName =
  | "microphone"
  | "camera"
  | "display-capture"
  | "notifications"
  | "clipboard-read"
  | "clipboard-write"
  | "geolocation";

/** `"Windows"` / `"macOS"` — the name an advisory would use. */
declare function __vn_permissions_platform(): string;
/** The four-state read, or `""` for a name that is not a permission. */
declare function __vn_permissions_query(name: string): VnPermissionState | "";
/** `[{ name, state, canRequest }]`, as JSON. */
declare function __vn_permissions_list(): VnJson;
/**
 * Starts a request. The JSON answer is one of three shapes:
 * `{ "error": … }` (not a permission name), `{ "state": … }` (settled
 * synchronously — nothing to ask), or `{ "token": n }` (the per-frame pump will
 * deliver the result to the dispatcher under that token).
 */
declare function __vn_permissions_request(name: string): VnJson;
/** Installs the callback the pump settles pending requests through. */
declare function __vn_permissions_set_dispatcher(
  fn: (token: number, state: VnPermissionState) => void,
): void;

// ---------------------------------------------------------------------------
// storage — crates/vn-storage/src/lib.rs (docs/STORAGE.md tier 2)
// ---------------------------------------------------------------------------

/** `{ path }` or `{ error }`. Creates nothing — a path is not a directory. */
declare function __vn_storage_user_folder(kind: string, subfolder: string): VnJson;
/** Points `localStorage` at `path`, creating it. `{ path }` or `{ error }`. */
declare function __vn_storage_set_folder(path: string, encrypted: boolean): VnJson;
/** `{ folder, file, encrypted, bytes, loaded, pending, provider, kinds, error }`. */
declare function __vn_storage_info(): VnJson;
/** Forces the coalesced write. `{ wrote: boolean }` or `{ error }`. */
declare function __vn_storage_flush(): VnJson;

// ---------------------------------------------------------------------------
// voice — crates/vn-voice/src/lib.rs (CONTRACT.md, docs/VOICE.md)
// ---------------------------------------------------------------------------

/** Starts an engine for one room. `0` means "could not start". */
declare function __vn_voice_create(optionsJson: VnJson): number;
/**
 * Posts one op to the room's actor and returns its request id. `0` is the
 * SYNCHRONOUS-REFUSAL sentinel — an unknown room id, or an engine whose channel
 * is closed — never a real request id, which is what lets the shim reject
 * immediately instead of leaving a promise pending forever.
 */
declare function __vn_voice_call(roomId: number, op: number, argJson: VnJson): number;
/** A live read of the actor's plain-data mirror. `""` for an unknown room. */
declare function __vn_voice_snapshot(roomId: number): VnJson;
/** Installs the one callback every room's results and events arrive through. */
declare function __vn_voice_set_dispatcher(fn: VnVoiceDispatcher): void;
/** Posts Shutdown and JOINS the thread, so no callback can still be in flight. */
declare function __vn_voice_destroy(roomId: number): void;

/**
 * `(roomId, kind, reqId, payload)`.
 *
 * `kind` 0 = a request RESOLVED (payload is its JSON result), 1 = a request
 * REJECTED (payload is the message); anything >= 16 is a room EVENT code from
 * `crates/vn-voice/src/proto.rs` and `reqId` is 0.
 */
type VnVoiceDispatcher = (
  roomId: number,
  kind: number,
  reqId: number,
  payload: string,
) => void;

// --- what crosses in the payload -------------------------------------------
//
// Read off `crates/vn-voice/src/engine.rs`'s `Obj::new().str(…)` builders, not
// off the shim: these are what the ACTOR emits, and the point of writing them
// down is that a disagreement between the two sides becomes a type error
// instead of a field that silently reads `undefined`.
//
// Every field is optional here even where engine.rs always emits it, because
// the shim guards each one with `!== undefined` and the unit harness sends
// partial payloads on purpose. Where the guarantee matters it is stated in the
// doc comment rather than in the `?`.

/** `kind_str` — engine.rs. Never "unknown" on the wire; the shim's own
 *  `Track.Kind.Unknown` is a local default, not something the actor sends. */
type VnVoiceTrackKind = "audio" | "video" | "unknown";
/** `source_str` — engine.rs. */
type VnVoiceTrackSource =
  | "camera"
  | "microphone"
  | "screen_share"
  | "screen_share_audio"
  | "unknown";
/** The shim's own projection; the actor never sends a stream state. */
type VnVoiceStreamState = "active" | "paused" | "unknown";
/** `state_str` — engine.rs. "signalReconnecting" is the shim's own enum member
 *  and is never emitted: the Rust SDK exposes three states (voice:signal-substates). */
type VnVoiceConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "signalReconnecting";
/** `quality_str` — engine.rs emits only the first four. "unknown" is the shim's
 *  own default, and `""` (below) is what a default-constructed roster carries. */
type VnVoiceConnectionQuality = "excellent" | "good" | "poor" | "lost" | "unknown";

/** `TrackState::json()` — all eight keys are always present on the wire. */
interface VnVoiceTrackState {
  sid: string;
  name?: string;
  /** Present on the track itself, in addition to the event's flat `identity`. */
  ownerIdentity?: string;
  kind?: VnVoiceTrackKind;
  source?: VnVoiceTrackSource;
  muted?: boolean;
  /** Always `true` for a LOCAL publication, whatever the SDK thinks. */
  subscribed?: boolean;
  isLocal?: boolean;
}

/** `ParticipantState::json()` — all eleven keys are always present on the wire. */
interface VnVoiceParticipantState {
  identity?: string;
  sid?: string;
  name?: string;
  /** `""` when unset, never null. */
  metadata?: string;
  /** Always an object, possibly empty. A FRESH object every event — see the
   *  ParticipantAttributesChanged note in the conversion report. */
  attributes?: Record<string, string>;
  isLocal?: boolean;
  isSpeaking?: boolean;
  /** DERIVED by the actor: "no unmuted audio publication exists". */
  isMuted?: boolean;
  audioLevel?: number;
  /** `""` from a `ParticipantState::default()` — i.e. the local participant
   *  before connect, and every participant after `reset_roster`. */
  connectionQuality?: VnVoiceConnectionQuality | "";
  tracks?: VnVoiceTrackState[];
  /**
   * `undefined`, and that is the whole type. `ParticipantState::json()` has no
   * `permissions` key — the actor's mirror never built one — but the shim reads
   * `state.permissions` to fill livekit-client's `Participant.permissions`.
   * Typed as `undefined` rather than dropped so the read still compiles and
   * says out loud what it produces: nothing, for every participant, always.
   */
  permissions?: undefined;
}

/** The union of every room event's payload (dispatcher `kind >= 16`). One flat
 *  interface rather than a discriminated union because the discriminant is the
 *  dispatcher's separate `kind` argument, not a field. The comment on each
 *  field names the event code that sends it. */
interface VnVoiceEventData {
  /** 17 CONNECTION_STATE. */
  state?: VnVoiceConnectionState;
  /** 18 CONNECTED — the room NAME. */
  room?: string;
  /** 18 CONNECTED. */
  localParticipant?: VnVoiceParticipantState;
  /** 22 PARTICIPANT_CONNECTED, 23 PARTICIPANT_DISCONNECTED, 25 PARTICIPANT_UPDATED. */
  participant?: VnVoiceParticipantState;
  /** 24 PARTICIPANT_ACTIVE, 26-34 track events, 36 PARTICIPANT_STATE,
   *  37 CONNECTION_QUALITY, 38 DATA_RECEIVED (`""` for an anonymous packet). */
  identity?: string;
  /** 19 DISCONNECTED. NOT one of the exported `DisconnectReason` values on
   *  every path — see the conversion report. */
  reason?: string;
  /** 39 CONNECT_FAILED. Nothing in the shim reads it. */
  error?: string;
  /** 36 PARTICIPANT_STATE. */
  isSpeaking?: boolean;
  /** 36 PARTICIPANT_STATE. */
  audioLevel?: number;
  /** 36 PARTICIPANT_STATE. */
  isMuted?: boolean;
  /** 36 PARTICIPANT_STATE. */
  connectionQuality?: VnVoiceConnectionQuality | "";
  /** 37 CONNECTION_QUALITY. */
  quality?: VnVoiceConnectionQuality;
  /** 35 ACTIVE_SPEAKERS. Unordered — a HashSet on the far side. */
  identities?: string[];
  /** 26-34, the track events. */
  track?: VnVoiceTrackState;
  /** 34 LOCAL_TRACK_REPUBLISHED, and only when the mic sid actually changed. */
  previousSid?: string;
  /** 38 DATA_RECEIVED — base64. */
  payload?: string;
  /** 38 DATA_RECEIVED. */
  reliable?: boolean;
  /** 38 DATA_RECEIVED — `""` when the packet carried none, never null. */
  topic?: string;
  /** 40 RTT. `-1` means unknown. */
  rttMs?: number;
}

/** `Roster::json()` — what `__vn_voice_snapshot` answers. All five keys are
 *  always present; `""` (not `"{}"`) comes back for an unknown room id. */
interface VnVoiceSnapshot {
  state?: VnVoiceConnectionState;
  /** The room NAME. `Room._seed` does not read it. */
  room?: string;
  rttMs?: number;
  localParticipant?: VnVoiceParticipantState;
  participants?: VnVoiceParticipantState[];
}

// ---------------------------------------------------------------------------
// the `__VN_*` globals — not host FUNCTIONS, but part of the same contract
// ---------------------------------------------------------------------------

/**
 * Every SFC's `<style>` block, collected at module-eval time by the builder and
 * joined into the first stylesheet by `mount()` (build.ts emits the pushes).
 */
declare var __VN_CSS__: string[] | undefined;

/**
 * Installed by `mount()`; called by the host before it re-evals a new bundle.
 * Unmounts the app and frees every node the mirror still holds.
 */
declare var __VN_TEARDOWN__: (() => void) | undefined;

/**
 * The windowing host (`crates/vn-app/src/window.prelude.js`, docs/WINDOWING.md
 * tier 3). Absent in the unit-test harness and in any embedder that does not
 * install the prelude — `runtime.ts` exports an inert `appWindow` then, and
 * says so once.
 */
declare var __VN_WINDOW__: VnWindowHost | undefined;

interface VnWindowHost {
  appWindow: VnAppWindow;
  defineWindow: (options: Record<string, unknown>) => void;
}

/** `{ x, y, width, height }`, in physical window coordinates. */
interface VnWindowRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * `appWindow.chrome` — the chrome-mode read, so an app can branch without a
 * media query. `vn_app::window` builds it field by field, so the shape is fixed
 * and small; it was a bag of `unknown` until an app tried to use it.
 *
 * `client` is the window's CLIENT AREA, and `contentInset` is how far the app's
 * viewport is inset inside it by host-drawn chrome — `inner* === client* -
 * inset` by construction, and in custom-chrome mode every inset is zero.
 */
interface VnWindowChrome {
  titleBarStyle: string;
  displayMode: string;
  titleBarHeight: number;
  fallbackTitleBar: boolean;
  overlayVisible: boolean;
  /** Whether the HOST draws the min/max/close cluster (`titleBarOverlay`). */
  hostControls: boolean;
  client: { width: number; height: number };
  contentInset: { left: number; top: number; right: number; bottom: number };
}

/**
 * The window facade the prelude builds. An `EventTarget` first — apps listen
 * for `resize`/`move`/`focus` on it — with the Tauri/Electron-shaped verbs
 * bolted on. Every verb is SYNCHRONOUS and returns `undefined`: JS and the UI
 * thread are the same thread here, so a promise would lie about ordering
 * (`await undefined` is legal, so ported `await appWindow.maximize()` still
 * runs).
 */
interface VnAppWindow extends VnEventTarget {
  minimize(): void;
  maximize(): void;
  unmaximize(): void;
  toggleMaximize(): void;
  restore(): void;
  close(): void;
  center(): void;
  startDragging(): void;
  startResizeDragging(): void;
  setFullscreen(on: boolean): void;
  isMaximized(): boolean;
  isMinimized(): boolean;
  isFullscreen(): boolean;
  isFocused(): boolean;
  getBounds(): VnWindowRect;
  getContentBounds(): VnWindowRect;
  setBounds(rect: Partial<VnWindowRect>): void;
  setContentBounds(rect: Partial<VnWindowRect>): void;
  setTitleBarOverlay(overlay: boolean | null | Record<string, unknown>): void;
  title: string;
  backdrop: string;
  alwaysOnTop: boolean;
  readonly scaleFactor: number;
  readonly chrome: VnWindowChrome;
  /** Windows taskbar integration — present, advisory-only in v1. */
  setProgressBar(...args: unknown[]): void;
  setOverlayIcon(...args: unknown[]): void;
  setBadgeCount(...args: unknown[]): void;
  setSkipTaskbar(...args: unknown[]): void;
  setIcon(...args: unknown[]): void;
}

/**
 * Sticky activation (docs/WINDOWING.md §1.9). The prelude arms it from the
 * engine's own EventTarget, which gpui events never reach — so the host's
 * dispatcher calls this instead, on `keydown`/`mousedown`. Without it
 * `beforeunload` could never veto a close, because the window would never count
 * as having been interacted with.
 */
declare var __vnStickyActivation: (() => void) | undefined;

/** Vue's dev-only HMR runtime, which `loader.ts` drives. */
declare var __VUE_HMR_RUNTIME__: VnVueHmrRuntime | undefined;

interface VnVueHmrRuntime {
  createRecord(id: string, def: unknown): boolean;
  rerender(id: string, render: unknown): void;
  reload(id: string, def: unknown): void;
}
