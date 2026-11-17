// Names cross the host boundary once; ids after.
const interned = new Map<string, SkNameId>();
export const I = (s: string): SkNameId => {
  let v = interned.get(s);
  if (v === undefined) {
    v = __sk_intern(s);
    interned.set(s, v);
  }
  return v;
};

// Keep in lockstep with EV_* in sk-tree's tree.rs.
export const EV_CODES = {
  click: 1,
  mousedown: 2,
  mouseup: 3,
  wheel: 4,
  keydown: 5,
  keyup: 6,
  focus: 7,
  blur: 8,
  input: 9,
  scroll: 10,
  complete: 11,
  loopComplete: 12,
  enterFrame: 13,
  // Pointer family: the host sources only mousemove/mouseenter/mouseleave; the rest of the W3C set is
  // DERIVED from them and mousedown/mouseup (`DERIVED`).
  mousemove: 14,
  mouseenter: 15,
  mouseleave: 16,
  loadstart: 17,
  loadedmetadata: 18,
  loadeddata: 19,
  canplay: 20,
  canplaythrough: 21,
  play: 22,
  playing: 23,
  pause: 24,
  seeking: 25,
  seeked: 26,
  // The host throttles it to the spec's 250 ms (MF raises it at the display's rate).
  timeupdate: 27,
  durationchange: 28,
  ratechange: 29,
  volumechange: 30,
  waiting: 31,
  ended: 32,
  // `a` is MediaError.code (1-4), `key` the advisory naming the missing codec.
  error: 33,
  emptied: 34,
  // `a` = elapsedTime, `key` = the property / @keyframes name.
  transitionend: 35,
  animationstart: 36,
  animationend: 37,
  animationcancel: 38,
  paste: 39,
  copy: 40,
  cut: 41,
  // `contextmenu`: right-button release, or Menu key / Shift+F10. `load`: an `<img>` has its picture (a/b = natural
  // size, `key` = src; a failure is `error` with the same `key`). `pointercancel`: the host gave up on a press.
  contextmenu: 42,
  load: 43,
  pointercancel: 44,
  // Document text selection. `selectstart` is cancelable, at the press target. `selectionchange` arrives at the
  // ROOT and is retargeted to `document`, where a browser fires it.
  selectstart: 45,
  selectionchange: 46,
  auxclick: 47,
  // Text-field edits: `beforeinput` precedes an edit and cancels it (an IME's excepted); `a` is the `EDIT_TYPES` code,
  // `b` 1 while an IME composes, the eighth argument the `data` string. `input` (9) adds the new value in `key`.
  beforeinput: 48,
  compositionstart: 49,
  compositionupdate: 50,
  compositionend: 51,
  // An `<iframe>` document's postMessage, delivered at the iframe node and retargeted to `window`. `key` is
  // `{"o": origin, "d": wire}`, the payload in `iframe.ts`'s wire form.
  message: 52,
  // The document's pointer lock, delivered at node 0 and fired at `document`. `a` is the request it settles (0: none),
  // an error's `key` "Name: message".
  pointerlockchange: 53,
  pointerlockerror: 54,
  // `<sk-surface>`: the painted size (a × b physical px, c the scale), the first frame shown (a × b), the frame gone
  // (`key` "cleared" or "device"), the display (a headroom, b potential headroom, c refresh Hz, d SDR white nits, 0
  // unknown; `key` "hdr", "vrr" or "fixed", space-separated).
  surfaceresize: 55,
  surfaceready: 56,
  surfacelost: 57,
  surfacedisplaychange: 58,
} as const;

/** `InputEvent.inputType` by host code: indexes sk-elements' `text_input::InputType` (lockstep, test/protocol-codes.test.ts). 0: none. */
export const EDIT_TYPES = [
  "",
  "insertText",
  "insertLineBreak",
  "insertFromPaste",
  "insertCompositionText",
  "deleteContentBackward",
  "deleteContentForward",
  "deleteWordBackward",
  "deleteWordForward",
  "deleteSoftLineBackward",
  "deleteSoftLineForward",
  "deleteByCut",
  "historyUndo",
  "historyRedo",
] as const;

type HostEventName = keyof typeof EV_CODES;
// Events synthesized in JS from one the host delivers. `pre: true` fires BEFORE its source (W3C pointer order:
// `pointerdown` precedes `mousedown`); `focusin`/`focusout` bubble and fire after `focus`/`blur`.
// A `pointerdown` listener registers the host's `mousedown` bit and receives the mouse event's payload.
export const DERIVED = {
  pointerdown: { from: "mousedown", pre: true },
  pointerup: { from: "mouseup", pre: true },
  pointermove: { from: "mousemove", pre: true },
  pointerenter: { from: "mouseenter", pre: true },
  pointerleave: { from: "mouseleave", pre: true },
  focusin: { from: "focus", pre: false },
  focusout: { from: "blur", pre: false },
  // After the click with `detail` 2 and the primary button: only the second click of a pair makes one.
  dblclick: { from: "click", pre: false },
} as const satisfies Record<string, { from: HostEventName; pre: boolean }>;

export type DerivedEventName = keyof typeof DERIVED;

export const hostCode = (name: string): SkEventCode | undefined =>
  (EV_CODES as Record<string, SkEventCode | undefined>)[name] ??
  (EV_CODES as Record<string, SkEventCode | undefined>)[
    (DERIVED as Record<string, { from: HostEventName } | undefined>)[name]?.from as string
  ];
export const NAMES_BY_CODE: Record<number, string[]> = (() => {
  const out: Record<number, string[]> = {};
  const add = (code: SkEventCode, name: string) => (out[code] ??= []).push(name);
  for (const name in EV_CODES) add(EV_CODES[name as HostEventName], name);
  for (const name in DERIVED) add(EV_CODES[DERIVED[name as DerivedEventName].from], name);
  return out;
})();
export const DERIVED_OF: Record<string, string[] | undefined> = (() => {
  const out: Record<string, string[]> = {};
  for (const name in DERIVED) (out[DERIVED[name as DerivedEventName].from] ??= []).push(name);
  return out;
})();
export const POINTER_EVENTS = new Set<string>([
  "pointerdown",
  "pointerup",
  "pointermove",
  "pointerenter",
  "pointerleave",
  "pointercancel",
  "gotpointercapture",
  "lostpointercapture",
]);
export const NOT_CANCELABLE = new Set<string>(["pointercancel", "gotpointercapture", "lostpointercapture", "select"]);
// Vue hyphenates an `@camelCase` listener prop (`onLoopComplete` -> "loop-complete"); accept both spellings and
// fold them onto the camelCase name.
const EVENT_ALIASES: Record<string, string | undefined> = {
  "loop-complete": "loopComplete",
  loopcomplete: "loopComplete",
  "enter-frame": "enterFrame",
  enterframe: "enterFrame",
};
export const canonicalEvent = (name: string): string => EVENT_ALIASES[name] ?? name;
// These fire at the target only (`scroll` bubbles from the document, never from an element).
export const NO_BUBBLE = new Set<string>([
  "focus",
  "blur",
  "scroll",
  "selectionchange",
  "complete",
  "loopComplete",
  "enterFrame",
  // enter/leave do not bubble; getting that wrong closes a menu as soon as the pointer crosses one of its items.
  "mouseenter",
  "mouseleave",
  "pointerenter",
  "pointerleave",
  // Media events fire at the target only; else a wrapper's `@error` would catch a `<video>`'s failed load.
  "loadstart",
  "loadedmetadata",
  "loadeddata",
  "canplay",
  "canplaythrough",
  "play",
  "playing",
  "pause",
  "seeking",
  "seeked",
  "timeupdate",
  "durationchange",
  "ratechange",
  "volumechange",
  "waiting",
  "ended",
  "error",
  "emptied",
  "load",
  "surfaceresize",
  "surfaceready",
  "surfacelost",
  "surfacedisplaychange",
]);
export const MEDIA_EVENTS = new Set<string>([
  "loadstart",
  "loadedmetadata",
  "loadeddata",
  "canplay",
  "canplaythrough",
  "play",
  "playing",
  "pause",
  "seeking",
  "seeked",
  "timeupdate",
  "durationchange",
  "ratechange",
  "volumechange",
  "waiting",
  "ended",
  "error",
  "emptied",
]);
