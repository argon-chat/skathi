import type { Platform, Release, Status, ValueSupport } from "../css/entries";
import { bcd } from "./bcd";

export type Kind = "interface" | "method" | "property" | "event" | "function" | "element" | "api" | "feature";

export const categories = [
  "Language and scheduling",
  "DOM and events",
  "Networking",
  "Streams, binary data and encoding",
  "Storage and files",
  "Graphics and media",
  "System integration",
] as const;
export type Category = (typeof categories)[number];

/** Section prose from the coverage page that applies to the whole category. Inline code is Markdown. */
export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {};

export interface WebEntry {
  slug: string;
  name: string;
  kind: Kind;
  category: Category;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  experimental?: boolean;
  /** Deprecated on the web (BCD), as opposed to `status: "deprecated"` in Skathi. */
  webDeprecated?: boolean;
  skathiOnly?: boolean;
  mdn?: string;
  values?: ValueSupport[];
  /** What works and how it behaves. */
  notes?: string[];
  /** What differs from browsers. */
  limitations?: string[];
  /** What to write instead. */
  workaround?: string;
  related?: string[];
  aliases?: string[];
}

type Extra = Partial<
  Pick<WebEntry, "values" | "notes" | "limitations" | "workaround" | "related" | "aliases" | "skathiOnly" | "since">
> & { slug?: string; platforms?: Record<Platform, Status> };

interface Draft extends Extra {
  name: string;
  kind: Kind;
  status: Status;
  summary: string;
}

const OK = "supported";
const PART = "partial";
const PLAN = "planned";
const HOLD = "on-hold";
const NO = "not-planned";
const GONE = "deprecated";

const FIRST_RELEASE: Release = "0.2.0";
const NOT_ON_LINUX: Record<Platform, Status> = { windows: OK, macos: OK, linux: PLAN };

const draft =
  (kind: Kind) =>
  (name: string, status: Status, summary: string, x: Extra = {}): Draft => ({ name, kind, status, summary, ...x });
const iface = draft("interface");
const method = draft("method");
const prop = draft("property");
const fn = draft("function");
const named =
  (kind: Kind) =>
  (slug: string, name: string, status: Status, summary: string, x: Extra = {}): Draft => ({
    slug,
    name,
    kind,
    status,
    summary,
    ...x,
  });
const api = named("api");
const feat = named("feature");
const el = named("element");
const ev = named("event");

function slugFor(d: Draft): string {
  if (d.slug !== undefined) return d.slug;
  return d.name
    .replace(/\(\)$/, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1-$2")
    .replace(/[^A-Za-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .toLowerCase();
}

function section(category: Category, drafts: readonly Draft[]): WebEntry[] {
  return drafts.map(({ platforms, ...d }) => {
    const slug = slugFor(d);
    const e: WebEntry = {
      ...d,
      slug,
      category,
      platforms: platforms ?? { windows: d.status, macos: d.status, linux: d.status },
    };
    if (d.status === OK || d.status === PART) e.since = d.since ?? FIRST_RELEASE;
    const info = bcd[slug];
    if (info?.mdn !== undefined) e.mdn = info.mdn;
    if (info?.experimental) e.experimental = true;
    if (info?.deprecated) e.webDeprecated = true;
    return e;
  });
}

const TIMERS = ["set-timeout", "set-interval", "queue-microtask"];
const FRAME_LOOP = "Driven by the window's frame loop.";
const ERROR_REPORT = "Uncanceled errors reach the host's error report.";
const ERRORS = ["report-error", "error-event", "onerror"];
const PERFORMANCE = ["performance-now", "performance-time-origin"];
const PERFORMANCE_TIMELINE = ["performance-mark", "performance-measure", "performance-observer"];
const WORKERS_HOLD = "On hold: waiting for multithreading in Bun and JavaScriptCore.";
const WORKERS = ["web-workers", "shared-worker", "message-channel"];

const languageAndScheduling = section("Language and scheduling", [
  fn("setTimeout", OK, "Runs a function once after a delay.", {
    related: TIMERS.filter((s) => s !== "set-timeout"),
    aliases: ["clearTimeout", "timers"],
  }),
  fn("setInterval", OK, "Runs a function repeatedly at a fixed interval.", {
    related: TIMERS.filter((s) => s !== "set-interval"),
    aliases: ["clearInterval", "timers"],
  }),
  fn("queueMicrotask", OK, "Queues a function to run as a microtask, before control returns to the event loop.", {
    related: TIMERS.filter((s) => s !== "queue-microtask"),
    aliases: ["microtask"],
  }),
  fn("requestAnimationFrame", OK, "Calls a function before the next repaint.", {
    notes: [FRAME_LOOP],
    related: ["request-idle-callback"],
    aliases: ["rAF", "cancelAnimationFrame"],
  }),
  fn("requestIdleCallback", OK, "Calls a function when the event loop is idle.", {
    notes: [FRAME_LOOP],
    related: ["request-animation-frame"],
    aliases: ["cancelIdleCallback"],
  }),
  fn("structuredClone", OK, "Deep-copies a value with the structured clone algorithm.", {
    notes: ["`FileSystemHandle` objects can be cloned too."],
    related: ["file-system-access"],
  }),
  fn("reportError", OK, "Reports an error to the global error handlers as if it were uncaught.", {
    notes: [ERROR_REPORT],
    related: ERRORS.filter((s) => s !== "report-error"),
  }),
  iface("ErrorEvent", OK, "The event dispatched for a script error, with its message, source and location.", {
    notes: [ERROR_REPORT],
    related: ERRORS.filter((s) => s !== "error-event"),
  }),
  ev("onerror", "onerror", PART, "The global handler for the `error` event fired by uncaught script errors.", {
    notes: [ERROR_REPORT],
    limitations: [
      "Only `reportError` and the `ResizeObserver` loop error reach it. An exception thrown by an event listener, a timer, `requestAnimationFrame` or an idle callback goes to the host's error log instead.",
      'It runs after every listener added with `addEventListener("error", ...)`, wherever it was assigned.',
    ],
    related: ERRORS.filter((s) => s !== "onerror"),
    aliases: ["window.onerror", "error event"],
  }),
  method("performance.now()", OK, "Returns a high-resolution timestamp relative to the time origin.", {
    related: PERFORMANCE.filter((s) => s !== "performance-now"),
  }),
  prop("performance.timeOrigin", OK, "The high-resolution timestamp that `performance.now()` counts from.", {
    related: PERFORMANCE.filter((s) => s !== "performance-time-origin"),
  }),
  method("performance.mark", NO, "Adds a named timestamp to the performance timeline.", {
    related: PERFORMANCE_TIMELINE.filter((s) => s !== "performance-mark"),
  }),
  method("performance.measure", NO, "Adds a named duration between two marks to the performance timeline.", {
    related: PERFORMANCE_TIMELINE.filter((s) => s !== "performance-measure"),
  }),
  iface("PerformanceObserver", NO, "Observes new entries in the performance timeline.", {
    related: PERFORMANCE_TIMELINE.filter((s) => s !== "performance-observer"),
  }),
  api("web-workers", "Web Workers", HOLD, "Runs a script in a background thread (`Worker`).", {
    limitations: [WORKERS_HOLD],
    related: WORKERS.filter((s) => s !== "web-workers"),
    aliases: ["Worker", "new Worker", "dedicated worker"],
  }),
  iface("SharedWorker", HOLD, "A worker shared by several browsing contexts.", {
    limitations: [WORKERS_HOLD],
    related: WORKERS.filter((s) => s !== "shared-worker"),
  }),
  iface("MessageChannel", HOLD, "A pair of connected ports for passing messages between contexts.", {
    limitations: [WORKERS_HOLD],
    related: WORKERS.filter((s) => s !== "message-channel"),
    aliases: ["MessagePort"],
  }),
  iface("BroadcastChannel", OK, "Sends messages to every listener on a named channel.", {
    notes: ["Messages are delivered within the app."],
  }),
]);

const DOM_NOTE = "The DOM is Skathi's element tree, rendered natively.";
const DOM = ["elements-and-attributes", "get-computed-style", "element-from-point", "scroll-into-view"];
const EVENTS = [
  "event",
  "custom-event",
  "event-target",
  "ui-event",
  "mouse-event",
  "pointer-event",
  "keyboard-event",
  "wheel-event",
  "gesture-event",
  "focus-event",
  "input-event",
  "drag-event",
];
const OBSERVER_NOTE = "Callbacks run after layout, before paint, as in a browser.";
const DIALOG_WORKAROUND = 'Ask in the app\'s own UI, such as a Vue component with `role="dialog"`.';
const DIALOGS = ["alert", "confirm", "prompt"];
const NO_SHADOW = "Use Vue components.";

const eventIface = (
  name: string,
  summary: string,
  aliases: string[] = [],
  status: Status = OK,
  limitations?: string[],
) => {
  const slug = slugFor({ name, kind: "interface", status, summary });
  return iface(name, status, summary, {
    related: EVENTS.filter((s) => s !== slug),
    ...(aliases.length > 0 ? { aliases } : {}),
    ...(limitations ? { limitations } : {}),
  });
};

const domAndEvents = section("DOM and events", [
  api(
    "elements-and-attributes",
    "Elements and attributes",
    OK,
    "The document tree: elements, their attributes and the methods to create, find and change them.",
    {
      notes: [DOM_NOTE],
      related: DOM.filter((s) => s !== "elements-and-attributes"),
      aliases: ["DOM", "Element", "attributes", "setAttribute", "getAttribute"],
    },
  ),
  el("input-type-color", "<input type=color>", OK, "A form control that lets the user pick a color.", {
    since: "0.5.0",
    notes: [
      "A click, Enter, Space or `showPicker()` opens a picker Skathi draws under the input: a saturation and brightness square, a hue wheel or a triangle, a hue slider, fields in hex, RGB, HSL, HSV or OKLCH, and the app's recent colors, in the input's light or dark `color-scheme`. On Windows it is a popup window of its own and may extend past the app's window; on macOS and Linux it is drawn inside the window.",
      "Dragging, the arrow keys and typing fire `input` as the color moves; Enter or a click outside commits with `change`; Escape restores the value the picker opened with.",
      '`value` follows the HTML standard: `#rrggbb` by default, `color(srgb r g b / a)` with `alpha`, and `color(display-p3 r g b)` with `colorspace="display-p3"`. Any CSS color can be assigned and is converted to the input\'s space.',
      "The input fires a cancelable `showpicker` event before any picker opens: `preventDefault()` keeps it closed so the app can show its own.",
      '`"colorPicker": "system"` in `sk.config.json` opens the OS color dialog instead. It ignores `alpha`, `colorspace` and `hdr`; its pick is converted to the input\'s space.',
    ],
    values: [
      {
        value: "alpha",
        status: OK,
        note: "An alpha slider; `value` is `color(srgb r g b / a)` with 8-bit channels, or the `display-p3` form with `/ a`.",
      },
      {
        value: 'colorspace="limited-srgb"',
        status: OK,
        note: "The default: sRGB with 8-bit channels.",
      },
      {
        value: 'colorspace="display-p3"',
        status: OK,
        note: "The picker works in Display P3 coordinates and `value` is `color(display-p3 r g b)`, unrounded. Swatches are painted as the window shows them.",
      },
      {
        value: "hdr",
        status: OK,
        note: "Skathi only. An intensity in stops, −10 to +10, multiplies the color; `value` is linear light that can pass 1: `color(srgb-linear r g b)`, or `color(display-p3-linear r g b)` with `display-p3`.",
      },
    ],
    limitations: ["No eyedropper."],
    related: ["html-input-element-show-picker", "showpicker"],
    aliases: ["color input", "color picker", "color well"],
  }),
  method("HTMLInputElement.showPicker()", OK, "Opens an input's picker from script.", {
    since: "0.5.0",
    notes: [
      "Opens the color picker of `<input type=color>` and the file dialog of `<input type=file>`, after the `showpicker` event of a color input.",
      "Needs a user gesture: without one it throws a `NotAllowedError`, and on a disabled input an `InvalidStateError`.",
    ],
    limitations: ["Other input types have no picker: the call does nothing, with an advisory."],
    related: ["input-type-color", "input-type-file", "showpicker"],
  }),
  ev(
    "showpicker",
    "showpicker",
    OK,
    "Fired at a color input before its picker opens; cancel it to show your own picker.",
    {
      since: "0.5.0",
      skathiOnly: true,
      notes: [
        "Bubbles and is cancelable. It fires for a click, Enter, Space, a `<label>` and `showPicker()`, after the user-gesture check.",
        "After `preventDefault()` nothing opens; set `value` and dispatch `input` and `change` from your own picker.",
      ],
      related: ["input-type-color", "html-input-element-show-picker"],
      aliases: ["onshowpicker", "custom color picker"],
    },
  ),
  method("getComputedStyle", PART, "Returns the resolved values of the CSS properties of an element.", {
    notes: [DOM_NOTE],
    limitations: [
      'Only a set of properties is published: `display`, `position`, `visibility`, `overflow`, `overflow-x`, `overflow-y`, `opacity`, `z-index`, `pointer-events`, `transform`, `backdrop-filter`, `animation-name`, `animation-duration`, `animation-delay`, `animation-fill-mode`, `transition-property`, `transition-duration`, `transition-delay`, `width`, `height`, `padding-*`, `margin-*`, `border-*-width`, `direction`, `font-size`, `color`, `text-shadow` and every custom property. Any other property reads `""`, with an advisory.',
      "`translate`, `rotate`, `scale`, `perspective`, `filter` and `contain` always read `none`, `will-change` and `scrollbar-gutter` read `auto`, `container-type` reads `normal` and `direction` reads `ltr`.",
    ],
    related: DOM.filter((s) => s !== "get-computed-style"),
    aliases: ["window.getComputedStyle"],
  }),
  method("elementFromPoint", OK, "Returns the topmost element at a point in the viewport.", {
    notes: [DOM_NOTE],
    related: DOM.filter((s) => s !== "element-from-point"),
    aliases: ["document.elementFromPoint"],
  }),
  method("scrollIntoView", OK, "Scrolls an element's ancestors so that the element is visible.", {
    notes: [DOM_NOTE],
    related: [...DOM.filter((s) => s !== "scroll-into-view"), "smooth-scrolling"],
    aliases: ["element.scrollIntoView"],
  }),
  eventIface("Event", "The base interface of every event dispatched in the DOM."),
  eventIface("CustomEvent", "An event that carries application data in its `detail`."),
  eventIface(
    "EventTarget",
    "An object that can receive events and have listeners for them.",
    ["addEventListener", "removeEventListener", "dispatchEvent"],
    PART,
    [
      "On an element, the `signal` option is ignored: aborting it leaves the listener in place, so call `removeEventListener`. On `window` and `document` it works.",
      'On an element, `addEventListener` with an all-lowercase name the host never delivers, such as `"beforeclose"`, adds nothing and logs an advisory. Name custom events with a dash, a dot or a capital, such as `"before-close"`.',
      "A template listener such as `@before-close` on a native element is dropped with an advisory. Add it with `addEventListener`, or use a component `emit`.",
    ],
  ),
  eventIface("UIEvent", "The base interface of user interface events.", ["UI events"]),
  eventIface("MouseEvent", "An event from a pointing device such as a mouse.", ["mouse events"]),
  eventIface(
    "PointerEvent",
    "An event from a pointing device, with its button state and position.",
    ["pointer events"],
    PART,
    [
      'Events from the host come from the mouse (`pointerType` `"mouse"`, `pointerId` `1`) or a pen (`"pen"`, `pointerId` `2`). There is no touch input. `isPrimary` is `true`, and `width` and `height` are `1`.',
      "A mouse reports pressure `0.5` while a button is down and `0` on release. A macOS Force Touch trackpad also reports measured pressure during a press.",
    ],
  ),
  prop("PointerEvent.pointerType", PART, 'The kind of device behind the event: `"mouse"`, `"pen"` or `"touch"`.', {
    since: "0.5.0",
    platforms: { windows: PART, macos: PART, linux: PART },
    notes: [
      'A stylus reports `"pen"` with `pointerId` `2`, the mouse `"mouse"` with `1`. Each is the primary pointer of its type.',
      "A pen hovering in range fires `pointerover`, `pointerenter` and `pointermove` with `buttons` `0`; leaving range fires `pointerout` and `pointerleave`.",
      "The compatibility mouse events follow a pen's pointer events, unless `pointerdown` called `preventDefault()`; then the stroke's `mousedown`, `mousemove` and `mouseup` are not fired.",
      "On Windows, pen input comes from `WM_POINTER` messages. Press-and-hold right-click, flicks and the Windows Ink feedback ripples are turned off for the window.",
      "On macOS, AppKit's tablet events are the pen, and leaving proximity ends its hover.",
      "On Linux, Wayland compositors that offer `zwp_tablet_v2` deliver pens.",
    ],
    limitations: [
      'No touch input: the host never reports `"touch"`.',
      "On Linux under X11, a pen moves the core pointer and arrives as the mouse.",
    ],
    related: ["pointer-event", "pointer-event-pressure", "pointer-event-get-coalesced-events"],
    aliases: ["pen", "stylus", "pointerId", "isPrimary"],
  }),
  prop("PointerEvent.pressure", PART, "How hard the pen tip or Force Touch trackpad is pressed, from 0 to 1.", {
    since: "0.5.0",
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: [
      "A pen reports `0` while hovering and on `pointerup`. A device without pressure data, such as the mouse, reports `0.5` while a button is down.",
      "On Windows, it is the pen's `POINTER_PEN_INFO` pressure, 0 to 1024, scaled to 0 to 1; a pen that does not report pressure reads `0.5` while touching.",
      "On macOS, a pen supplies its tablet pressure. A Force Touch trackpad reports a normal press in the lower half of the range and a force click in the upper half. A change in pressure alone fires a mouse `pointermove`.",
      "On Linux with Wayland, it is the tablet tool's pressure axis, 0 to 65535, scaled to 0 to 1.",
    ],
    limitations: ["On Linux under X11, a pen arrives as the mouse and reads `0.5` while pressed."],
    related: ["pointer-event-pointer-type", "pointer-event-tangential-pressure", "pointer-event-tilt-x"],
  }),
  prop("PointerEvent.tangentialPressure", PART, "The barrel pressure of an airbrush-style pen, from -1 to 1.", {
    since: "0.5.0",
    platforms: { windows: PART, macos: OK, linux: PART },
    notes: [
      "On macOS, it is the tablet event's `tangentialPressure`.",
      "On Linux with Wayland, it is the tablet tool's slider axis.",
    ],
    limitations: [
      "On Windows, pen input has no barrel pressure, so it is always `0`.",
      "On Linux under X11, a pen arrives as the mouse and reads `0`.",
    ],
    related: ["pointer-event-pressure"],
  }),
  prop("PointerEvent.tiltX", PART, "The pen's tilt toward the x and y axes, in whole degrees from -90 to 90.", {
    since: "0.5.0",
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: [
      "`tiltX` leans toward the right, `tiltY` toward the user. Both are `0` for an upright pen and for the mouse.",
      "On Windows, they are the pen's `POINTER_PEN_INFO` tilt; a pen without tilt sensors reads `0`.",
      "On macOS, they are the tablet event's `tilt`, scaled from -1..1 to degrees.",
      "On Linux with Wayland, they are the tablet tool's tilt axis.",
    ],
    limitations: ["On Linux under X11, a pen arrives as the mouse and reads `0`."],
    related: ["pointer-event-altitude-angle", "pointer-event-twist"],
    aliases: ["tiltY"],
  }),
  prop(
    "PointerEvent.altitudeAngle",
    PART,
    "The pen's angle above the surface and its direction around it, in radians.",
    {
      since: "0.5.0",
      platforms: { windows: OK, macos: OK, linux: PART },
      notes: [
        "`altitudeAngle` is π/2 for an upright pen and `0` for one lying flat; `azimuthAngle` runs clockwise from the positive x axis.",
        "Both are computed from `tiltX` and `tiltY`, as the specification converts them. A constructed event given only the angles gets the tilt from them.",
      ],
      limitations: ["On Linux under X11, a pen arrives as the mouse and reads π/2 and `0`."],
      related: ["pointer-event-tilt-x"],
      aliases: ["azimuthAngle"],
    },
  ),
  prop("PointerEvent.twist", PART, "The pen's rotation about its own axis, in whole degrees from 0 to 359.", {
    since: "0.5.0",
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: [
      "On Windows, it is the pen's `POINTER_PEN_INFO` rotation; most pens do not report one and read `0`.",
      "On macOS, it is the tablet event's `rotation`.",
      "On Linux with Wayland, it is the tablet tool's rotation axis.",
    ],
    limitations: ["On Linux under X11, a pen arrives as the mouse and reads `0`."],
    related: ["pointer-event-tilt-x"],
  }),
  method("PointerEvent.getCoalescedEvents()", OK, "Returns the readings a `pointermove` stands for, oldest first.", {
    since: "0.5.0",
    notes: [
      "Moves are delivered once a frame. The readings since the last one each come back as a `PointerEvent` with its own position, pressure, tilt, `movementX`/`movementY` and `offsetX`/`offsetY`; the last one matches the event itself.",
      "On Windows, a pen's readings include the ones Windows batched into a single message (`GetPointerPenInfoHistory`).",
      "The mouse's moves within a frame are its readings too.",
      "Each reading's `timeStamp` is when the hardware took it, on the `performance.now()` clock, so it is never later than the `pointermove`'s own. A pen's comes from the platform: on Windows the reading's performance counter (or its message time, when the device leaves that out), on macOS the tablet event's `timestamp`, on Wayland the tool frame's time.",
      "Events other than `pointermove` return an empty list.",
    ],
    limitations: ["A mouse reading's `timeStamp` is when the host received the move, not a hardware time."],
    related: ["pointer-event", "pointer-event-get-predicted-events", "pointerrawupdate"],
    aliases: ["coalescedEvents"],
  }),
  method("PointerEvent.getPredictedEvents()", OK, "Returns positions the pointer is expected to reach next.", {
    since: "0.5.0",
    notes: [
      "A `pointermove` from a pen or the mouse lists where the pointer is expected after its last reading: up to three predictions, 1/180 s apart, so at most about 17 ms ahead. Each has the predicted position, `movementX`/`movementY` from the one before and the predicted time as `timeStamp`; pressure, tilt and the buttons are the event's own.",
      "The predictor is the Kalman predictor of Google's ink stroke modeler, with the parameters of its tests. How many predictions a move gets depends on its confidence: it grows over a stroke's first 20 readings and falls when the pointer moves slower than about 3 cm/s, when the filter's estimate strays from the latest reading, and when the predicted path curves away from a straight line, so the list shortens or empties on slow movement, sharp turns and braking instead of overshooting.",
      "It starts once a stroke has four readings and starts over on a press, a release, leaving the window or a pause of more than 20 ms; until then the list is empty.",
      "A constructed event returns the `predictedEvents` it was given.",
    ],
    related: ["pointer-event-get-coalesced-events"],
    aliases: ["predictedEvents"],
  }),
  ev(
    "pointerrawupdate",
    "pointerrawupdate",
    NO,
    "Fires for every pointer reading, ahead of the frame's `pointermove`.",
    {
      notes: [
        "Skathi delivers pointer input once per frame and draws once per frame, so handling each reading as it arrives would not show it any sooner.",
      ],
      workaround:
        "Read every reading since the last frame, each with its own `timeStamp`, from `getCoalescedEvents()` on `pointermove`.",
      related: ["pointer-event-get-coalesced-events"],
    },
  ),
  eventIface("KeyboardEvent", "An event from a key press or release.", ["keyboard events"], PART, [
    "Desktop keydown and keyup report the layout's `key`, the physical `code`, and the held modifier flags. `location` distinguishes left (1), right (2), and keypad (3) keys. Other keys use 0. Held keydown events set `repeat`.",
    "Legacy `keyCode` and `charCode` are unavailable.",
  ]),
  eventIface("WheelEvent", "An event from a mouse wheel or similar input.", ["wheel events"]),
  iface("GestureEvent", PART, "A phased trackpad gesture with its cumulative scale and physical modifier keys.", {
    since: "0.6.0",
    aliases: ["pinch", "gesturestart", "gesturechange", "gestureend"],
    notes: [
      "Desktop trackpad magnification sends gesturestart, gesturechange and gestureend to the element where the gesture began. Read clientX and clientY for the current gesture position.",
      "A scale update sends Ctrl+wheel to the element under the gesture centre. Cancelling the associated gesture event suppresses that wheel, including when gestureend carries the last change in scale.",
    ],
    limitations: ["The native input backends currently report rotation as 0."],
    related: ["wheel-event"],
  }),
  eventIface("FocusEvent", "An event fired when an element gains or loses focus.", ["focus events"]),
  eventIface("InputEvent", "An event fired when editable content changes.", ["input events"]),
  eventIface("DragEvent", "An event of a drag-and-drop interaction.", ["drag events", "drag and drop"]),
  iface("TouchEvent", PART, "An event from a touch surface, with its list of touch points.", {
    notes: ["Constructible."],
    limitations: ["The host does not deliver touch input yet."],
    related: ["pointer-event"],
    aliases: ["touch events", "Touch", "TouchList"],
  }),
  iface("ResizeObserver", PART, "Reports changes to the size of an element.", {
    notes: [OBSERVER_NOTE],
    limitations: [
      "`contentRect`, `contentBoxSize` and `borderBoxSize` all report the border box, and `contentRect` is at `(0, 0)`.",
    ],
    related: ["intersection-observer", "mutation-observer"],
  }),
  iface("IntersectionObserver", PART, "Reports when an element enters or leaves the viewport or an ancestor.", {
    notes: [OBSERVER_NOTE],
    limitations: [
      "Only the `root` clips: a target scrolled out of an inner scroll container but still inside the window reads as intersecting. Pass that container as `root`.",
    ],
    related: ["resize-observer", "mutation-observer"],
  }),
  iface("MutationObserver", PART, "Reports changes to the DOM tree.", {
    notes: ["`childList` and `attributes` are observed."],
    limitations: ["`characterData` is not observed."],
    related: ["resize-observer", "intersection-observer"],
  }),
  iface("Selection", OK, "The range of text the user selected, or the caret position.", {
    related: ["range"],
    aliases: ["getSelection"],
  }),
  iface("Range", PART, "A fragment of the document between two boundary points.", {
    limitations: [
      "A `Range` is static: DOM changes after it was set do not move its boundaries.",
      "`getClientRects()` and `getBoundingClientRect()` return element boxes, not the boxes of the selected glyphs.",
    ],
    related: ["selection"],
  }),
  api("contenteditable", "contenteditable", NO, "Lets the user edit an element's content in place.", {
    notes: ["The attribute only makes the element focusable: typing, pasting and IME input change nothing."],
    limitations: [
      "`isContentEditable`, `document.execCommand()` and `document.designMode` do not exist.",
      "Editors built on contenteditable do not work: CodeMirror, ProseMirror, Tiptap, Lexical, Quill, Slate and the like.",
    ],
    workaround:
      "For formatted text use [`<sk-editor>`](/elements/sk-editor/): marks, inline elements as atoms, line blocks and transactions, with IME, undo and the clipboard. For plain text use `<input>` or `<textarea>`.",
    related: ["selection", "input-event"],
    aliases: [
      "contentEditable",
      "isContentEditable",
      "execCommand",
      "designMode",
      "rich text editor",
      "code editor",
      "CodeMirror",
      "ProseMirror",
      "Tiptap",
      "Lexical",
      "Quill",
      "Slate",
    ],
  }),
  method("matchMedia", OK, "Evaluates a media query and reports changes to its result.", {
    aliases: ["window.matchMedia", "MediaQueryList"],
  }),
  api("fullscreen-api", "Fullscreen API", OK, "Shows an element, or the whole window, in full screen.", {
    aliases: ["requestFullscreen", "exitFullscreen", "fullscreenElement", "fullscreen"],
  }),
  api(
    "web-animations",
    "Web Animations",
    OK,
    "Animates elements from script with keyframes and timing, and controls running animations.",
    {
      aliases: ["Animation", "KeyframeEffect", "element.animate", "animate()"],
      related: ["view-transitions"],
    },
  ),
  api("view-transitions", "View Transitions", PART, "Animates between two states of the document.", {
    limitations: ["Same-document transitions only, in the main window."],
    related: ["web-animations"],
    aliases: ["document.startViewTransition", "startViewTransition", "::view-transition-*", "ViewTransition"],
  }),
  feat("smooth-scrolling", "Smooth scrolling", OK, "Animates a scroll to its destination instead of jumping.", {
    notes: ["`scrollTo`, `scrollBy`, `scrollIntoView` and `scrollTop`/`scrollLeft` scroll smoothly."],
    related: ["scroll-into-view"],
    aliases: ['behavior: "smooth"', "scroll-behavior: smooth", "scrollTo", "scrollBy", "scrollTop", "scrollLeft"],
  }),
  api("pointer-lock", "Pointer Lock", PART, "Hides the cursor and reports raw mouse movement to one element.", {
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: ["Escape always releases it."],
    limitations: ["On Linux, X11 only."],
    aliases: ["requestPointerLock", "exitPointerLock", "pointerLockElement"],
  }),
  iface("FontFace", OK, "A font face loaded from a URL or binary data, for use in the document.", {
    related: ["document-fonts"],
  }),
  prop("document.fonts", OK, "The document's set of font faces, to add fonts and wait for them to load.", {
    related: ["font-face"],
    aliases: ["FontFaceSet"],
  }),
  iface("XMLSerializer", OK, "Serializes a DOM tree to an XML string.", {
    related: ["dom-parser"],
  }),
  iface("DOMParser", NO, "Parses an HTML or XML string into a document.", {
    related: ["xml-serializer"],
  }),
  api("custom-elements", "Custom elements", NO, "Defines new HTML elements with their own behavior.", {
    notes: [
      "A `customElements` registry exists, so code that probes it works: `get()` returns `undefined`, `getName()` returns `null`, and `define()` and `whenDefined()` fail with `NotSupportedError`. Svelte then sets attributes on `sk-*` tags as plain attributes.",
    ],
    workaround: NO_SHADOW,
    related: ["shadow-dom"],
    aliases: ["customElements", "customElements.define", "CustomElementRegistry", "web components"],
  }),
  api("shadow-dom", "Shadow DOM", NO, "Attaches an encapsulated DOM tree to an element.", {
    workaround: NO_SHADOW,
    related: ["custom-elements"],
    aliases: ["attachShadow", "ShadowRoot", "web components"],
  }),
  method("alert", NO, "Shows a message in a modal dialog.", {
    notes: ["Shows no dialog: it logs an advisory once and returns right away."],
    workaround: DIALOG_WORKAROUND,
    related: DIALOGS.filter((s) => s !== "alert"),
    aliases: ["window.alert"],
  }),
  method("confirm", NO, "Asks the user to confirm or cancel in a modal dialog.", {
    notes: ["Shows no dialog: it logs an advisory once and returns `false`, as if the user canceled."],
    workaround: DIALOG_WORKAROUND,
    related: DIALOGS.filter((s) => s !== "confirm"),
    aliases: ["window.confirm"],
  }),
  method("prompt", NO, "Asks the user for a line of text in a modal dialog.", {
    notes: ["Shows no dialog: it logs an advisory once and returns `null`, as if the user canceled."],
    workaround: DIALOG_WORKAROUND,
    related: DIALOGS.filter((s) => s !== "prompt"),
    aliases: ["window.prompt"],
  }),
  method("window.open", PART, "Opens a URL in a new browsing context.", {
    notes: ["Opens external URLs in the system browser."],
    workaround: "A second window is `<SkWindow>`.",
    aliases: ["open"],
  }),
]);

const FETCH_NOTE = "HTTP/1.1 and HTTP/2, with an HTTP cache and a cookie jar.";
const FETCH_LIMIT = "Response bodies are buffered: `res.body` streams a body that has already arrived in full.";
const STREAM_BODY =
  "A `ReadableStream` body is refused with a `TypeError` by `fetch`, `new Request()` and `new Response()`. Collect the chunks into a `Blob` and send that.";
const BODY_USED = "Reading `res.body` marks the body used at once, so a later `res.text()` or `res.json()` rejects.";
const FETCH = ["fetch", "request", "response", "headers"];
const fetchRelated = (slug: string) => FETCH.filter((s) => s !== slug);

const networking = section("Networking", [
  fn("fetch", OK, "Makes an HTTP request and returns a promise for its response.", {
    notes: [FETCH_NOTE],
    limitations: [FETCH_LIMIT, STREAM_BODY],
    related: [...fetchRelated("fetch"), "xml-http-request", "abort-controller"],
  }),
  iface("Request", OK, "An HTTP request, as passed to `fetch`.", {
    notes: [FETCH_NOTE],
    limitations: [STREAM_BODY],
    related: fetchRelated("request"),
  }),
  iface("Response", OK, "The response to an HTTP request, with its status, headers and body.", {
    notes: [FETCH_NOTE],
    limitations: [FETCH_LIMIT, STREAM_BODY, BODY_USED],
    related: fetchRelated("response"),
  }),
  iface("Headers", OK, "A list of HTTP headers of a request or response.", {
    notes: [FETCH_NOTE],
    related: fetchRelated("headers"),
  }),
  iface("XMLHttpRequest", OK, "Makes an HTTP request with the older event-based API.", {
    notes: ["With upload and download progress."],
    related: ["fetch"],
    aliases: ["XHR"],
  }),
  iface("WebSocket", OK, "A two-way connection to a server over the WebSocket protocol.", {
    related: ["web-socket-stream", "event-source"],
  }),
  iface("WebSocketStream", OK, "A WebSocket connection exposed as readable and writable streams.", {
    related: ["web-socket", "readable-stream"],
  }),
  iface("EventSource", OK, "Receives server-sent events over an HTTP connection.", {
    notes: ["Reconnects with `Last-Event-ID` and the server's `retry`.", "The stream is requested uncompressed."],
    related: ["web-socket"],
    aliases: ["server-sent events", "SSE"],
  }),
  iface("URL", OK, "Parses, builds and normalizes URLs.", {
    limitations: ["`URL.parse()` is missing: check with `URL.canParse()`, then call `new URL()`."],
    related: ["url-search-params", "url-pattern"],
  }),
  iface("URLSearchParams", OK, "Reads and writes the query string of a URL.", {
    related: ["url"],
  }),
  iface("URLPattern", PART, "Matches URLs against a pattern and extracts their parts.", {
    limitations: ["Custom groups use Rust `regex` syntax: lookaround and backreferences are a `TypeError`."],
    related: ["url"],
  }),
  feat("accept-language", "Accept-Language", OK, "The request header that lists the languages the user prefers.", {
    notes: ["Set from `navigator.languages`, weighted as Chrome does."],
    related: ["navigator-languages", "fetch"],
  }),
  method("navigator.sendBeacon", NO, "Sends a small amount of data to a server without waiting for a response.", {
    related: ["fetch"],
    aliases: ["sendBeacon"],
  }),
  prop("navigator.connection", PART, "Reports the type of the network connection and fires `change` when it changes.", {
    notes: [
      '`type` is `"ethernet"`, `"wifi"`, `"cellular"`, `"other"`, `"none"` or `"unknown"`: the same value as `network.type` in [skathi:network](/std/network/), read from the same system state.',
      "`change` fires when `type` changes. `onchange` works as well as `addEventListener`.",
    ],
    limitations: [
      "`effectiveType`, `downlink`, `downlinkMax` and `rtt` are absent: they need measurements of the connection, which Skathi does not make.",
      "`saveData` is absent: it is the user's choice to save data, which is not what the connection's cost says. For a connection billed by use, read `metered` from [skathi:network](/std/network/).",
    ],
    aliases: ["NetworkInformation", "Network Information API", "connection.type", "effectiveType", "saveData"],
  }),
  api("service-workers", "Service workers", NO, "Scripts that intercept a site's network requests, for offline use.", {
    related: ["cache-api"],
    aliases: ["ServiceWorker", "navigator.serviceWorker"],
  }),
  api("cache-api", "Cache API", NO, "Stores request and response pairs for offline use.", {
    related: ["service-workers"],
    aliases: ["Cache", "caches", "CacheStorage"],
  }),
]);

const STREAMS = [
  "readable-stream",
  "writable-stream",
  "transform-stream",
  "byte-length-queuing-strategy",
  "count-queuing-strategy",
];
const streamRelated = (slug: string) => STREAMS.filter((s) => s !== slug);

const streams = section("Streams, binary data and encoding", [
  iface("ReadableStream", PART, "A source of data that can be read in chunks.", {
    notes: ["`ReadableStream.from` is supported."],
    limitations: [
      'Byte streams are not supported: `type: "bytes"` and `getReader({ mode: "byob" })` throw a `TypeError`. Enqueue `Uint8Array` chunks on a default stream instead.',
    ],
    related: streamRelated("readable-stream"),
    aliases: ["ReadableStream.from", "streams"],
  }),
  iface("WritableStream", OK, "A destination that data can be written to in chunks.", {
    related: streamRelated("writable-stream"),
    aliases: ["streams"],
  }),
  iface("TransformStream", OK, "A pair of streams that transforms the chunks written to it.", {
    related: streamRelated("transform-stream"),
    aliases: ["streams"],
  }),
  iface("ByteLengthQueuingStrategy", OK, "A queuing strategy that counts the bytes of each chunk.", {
    related: streamRelated("byte-length-queuing-strategy"),
    aliases: ["queuing strategies"],
  }),
  iface("CountQueuingStrategy", OK, "A queuing strategy that counts chunks.", {
    related: streamRelated("count-queuing-strategy"),
    aliases: ["queuing strategies"],
  }),
  iface("CompressionStream", OK, "Compresses a stream of data with gzip or deflate.", {
    related: ["decompression-stream", "transform-stream"],
  }),
  iface("DecompressionStream", OK, "Decompresses a stream of gzip or deflate data.", {
    related: ["compression-stream", "transform-stream"],
  }),
  iface("TextEncoder", OK, "Encodes a string into UTF-8 bytes.", {
    related: ["text-decoder"],
  }),
  iface("TextDecoder", OK, "Decodes bytes in a text encoding into a string.", {
    related: ["text-encoder"],
  }),
  fn("atob", OK, "Decodes a Base64 string.", {
    related: ["btoa"],
    aliases: ["base64"],
  }),
  fn("btoa", OK, "Encodes a string as Base64.", {
    related: ["atob"],
    aliases: ["base64"],
  }),
  iface("Blob", OK, "Immutable raw data with a MIME type.", {
    related: ["file", "file-reader"],
  }),
  iface("File", OK, "A `Blob` with a file name and a modification time.", {
    related: ["blob", "file-list", "file-reader"],
  }),
  iface("FileList", OK, "The list of files chosen in a file input or dropped on an element.", {
    related: ["file", "input-type-file"],
  }),
  iface("FileReader", OK, "Reads the contents of a `Blob` or `File` asynchronously.", {
    related: ["blob", "file"],
  }),
  iface("FormData", PART, "A set of form fields and files, to send with a request.", {
    limitations: ["`new FormData(form)` throws a `TypeError`: build the entries with `append`."],
    related: ["fetch"],
  }),
  iface("AbortController", OK, "Signals cancellation to one or more operations.", {
    related: ["abort-signal"],
  }),
  iface("AbortSignal", OK, "The signal an `AbortController` passes to the operations it can cancel.", {
    notes: ["`AbortSignal.timeout` and `AbortSignal.any` are supported."],
    related: ["abort-controller"],
    aliases: ["AbortSignal.timeout", "AbortSignal.any"],
  }),
  method("crypto.getRandomValues", OK, "Fills a typed array with cryptographically strong random values.", {
    related: ["crypto-random-uuid", "subtle-crypto"],
    aliases: ["getRandomValues"],
  }),
  method("crypto.randomUUID", OK, "Returns a random version 4 UUID.", {
    related: ["crypto-get-random-values", "subtle-crypto"],
    aliases: ["randomUUID", "uuid"],
  }),
  iface("SubtleCrypto", PART, "Low-level cryptography under `crypto.subtle`, such as hashing data.", {
    notes: ["`digest` with `SHA-1`, `SHA-256`, `SHA-384` and `SHA-512`."],
    limitations: ["`sign`, `encrypt`, `importKey`, `generateKey` and the other methods are not defined."],
    related: ["crypto-get-random-values", "crypto-random-uuid"],
    aliases: ["crypto.subtle", "Web Crypto"],
  }),
]);

const PICKER_NOTE = "Native pickers.";

const storage = section("Storage and files", [
  prop("localStorage", OK, "Key-value string storage that persists across sessions.", {
    notes: ["`localStorage` can be encrypted (`appStorage`)."],
    related: ["session-storage", "indexed-db"],
    aliases: ["Storage", "appStorage", "Web Storage"],
  }),
  prop("sessionStorage", OK, "Key-value string storage that lasts for the session.", {
    related: ["local-storage"],
    aliases: ["Storage", "Web Storage"],
  }),
  api("indexed-db", "IndexedDB", OK, "A transactional database of structured data and files.", {
    related: ["local-storage", "navigator-storage"],
    aliases: ["indexedDB", "IDBFactory", "IDBDatabase"],
  }),
  prop("navigator.storage", OK, "The storage manager: quota, usage and persistence of the app's storage.", {
    related: ["indexed-db"],
    aliases: ["StorageManager"],
  }),
  api(
    "file-system-access",
    "File System Access",
    OK,
    "Opens files and folders on the user's disk through pickers, and reads and writes them through handles.",
    {
      notes: ["Restored handles are granted on a user activation without a prompt."],
      related: ["structured-clone", "input-type-file", "navigator-user-activation"],
      aliases: [
        "showOpenFilePicker",
        "showSaveFilePicker",
        "showDirectoryPicker",
        "FileSystemHandle",
        "FileSystemFileHandle",
        "FileSystemDirectoryHandle",
        "FileSystemWritableFileStream",
      ],
    },
  ),
  el("input-type-file", "<input type=file>", OK, "A form control that lets the user choose files.", {
    notes: [PICKER_NOTE],
    related: ["file-list", "a-download", "file-system-access"],
    aliases: ["file input"],
  }),
  el("a-download", "<a download>", OK, "A link that downloads its target instead of navigating to it.", {
    notes: [PICKER_NOTE],
    related: ["input-type-file"],
    aliases: ["download attribute"],
  }),
  prop("navigator.locks", NO, "Requests named locks to coordinate work across contexts.", {
    aliases: ["Web Locks", "LockManager"],
  }),
]);

const MIC_NOTE = "Microphone, behind the app's permission prompt.";
const MIC_LIMIT = "No camera yet.";
const CAPTURE = ["get-user-media", "media-devices", "media-stream"];
const MEDIA_SESSION_NOTE = "System media controls and media keys (Windows: the media flyout; macOS: Now Playing).";

const graphics = section("Graphics and media", [
  api("canvas-2d", "Canvas 2D", OK, "Draws shapes, text and images on a `<canvas>` from script.", {
    related: ["offscreen-canvas", "path2d", "image-data"],
    aliases: ["CanvasRenderingContext2D", "getContext('2d')", "<canvas>"],
  }),
  iface("OffscreenCanvas", OK, "A canvas that is not attached to the document.", {
    notes: [
      '`getContext("2d")` and `getContext("webgpu")`; `convertToBlob` and `transferToImageBitmap` work with either.',
    ],
    related: ["canvas-2d", "webgpu"],
  }),
  iface("Path2D", OK, "A reusable path for a 2D canvas context.", {
    slug: "path2d",
    related: ["canvas-2d"],
  }),
  iface("ImageData", OK, "The raw RGBA pixels of an area of a canvas.", {
    related: ["canvas-2d", "image-bitmap"],
  }),
  iface("ImageBitmap", OK, "A decoded image that can be drawn on a canvas without delay.", {
    related: ["create-image-bitmap", "image-data"],
  }),
  fn("createImageBitmap", PART, "Decodes an image source into an `ImageBitmap`.", {
    limitations: [
      "The options argument (`resizeWidth`, `resizeHeight`, `imageOrientation`, `premultiplyAlpha` and the rest) is ignored. Scale with `drawImage` on an `OffscreenCanvas`.",
      "Images are decoded on the UI thread. A picked file and a `blob:` URL are read off it; a Blob in memory, an `ArrayBuffer` or an `<img>` is read on it too.",
    ],
    related: ["image-bitmap"],
  }),
  iface("DOMMatrix", PART, "A 2D or 3D transformation matrix.", {
    limitations: [
      '`new DOMMatrix("rotate(45deg)")` and `setMatrixValue()` throw a `TypeError`: CSS transform strings are not parsed. Pass 6 or 16 numbers, or chain `translate`, `rotate` and `scale`.',
    ],
    related: ["dom-point"],
  }),
  iface("DOMPoint", OK, "A 2D or 3D point.", {
    related: ["dom-matrix"],
  }),
  api("webgpu", "WebGPU", PART, "Renders and computes on the GPU.", {
    notes: [
      "Typed by the official `@webgpu/types`, which `@skathi-native/runtime` depends on: an app needs no install or `tsconfig.json` change. What they declare that Skathi lacks is under Limitations.",
      "Backed by wgpu 30: DirectX 12 on Windows, Metal on macOS, Vulkan on Linux.",
    ],
    values: [
      {
        value: "createRenderBundleEncoder()",
        status: OK,
        since: "0.5.0",
        note: "With `executeBundles()`. A bundle replays the same draws as the pass would, and can be executed again in later passes.",
      },
      {
        value: "createQuerySet()",
        status: OK,
        since: "0.5.0",
        note: "Occlusion queries (`occlusionQuerySet`, `beginOcclusionQuery()`, `endOcclusionQuery()`) and timestamp queries (`timestampWrites` on render and compute passes), read with `resolveQuerySet()`.",
      },
      {
        value: "timestamp-query",
        status: OK,
        since: "0.5.0",
        note: "An optional feature: request it in `requiredFeatures`. Timestamps are in nanoseconds.",
      },
      {
        value: "drawIndirect()",
        status: OK,
        since: "0.5.0",
        note: "With `drawIndexedIndirect()`, on render passes and render bundle encoders.",
      },
      {
        value: "createRenderPipelineAsync()",
        status: OK,
        since: "0.5.0",
        note: "With `createComputePipelineAsync()`. The pipeline is created off the JS thread; a bad shader or descriptor rejects with a `GPUPipelineError` whose `reason` is `validation` or `internal`.",
      },
      {
        value: "pushDebugGroup()",
        status: OK,
        since: "0.5.0",
        note: "With `popDebugGroup()` and `insertDebugMarker()` on command encoders, passes and render bundle encoders. Unbalanced groups are validation errors. A bundle's groups are checked but not recorded.",
      },
      {
        value: "setImmediates()",
        status: PART,
        since: "0.5.0",
        note: "Immediate data for `var<immediate>`, sized by the pipeline layout's `immediateSize`. `limits.maxImmediateSize` is 64 unless requested higher, up to the adapter's. On Windows the `var<immediate>` must be a struct: wgpu's HLSL output rejects other types, and pipeline creation fails with an internal error.",
      },
      {
        value: "label",
        status: OK,
        since: "0.5.0",
        note: "Every object has a readable, writable `label`, starting as the descriptor's.",
      },
      {
        value: "onuncapturederror",
        status: OK,
        since: "0.5.0",
        note: 'The same events as `addEventListener("uncapturederror", …)`.',
      },
      {
        value: "device.adapterInfo",
        status: OK,
        since: "0.5.0",
        note: "`subgroupMinSize` and `subgroupMaxSize` are 4 and 128, the specification's values while `subgroups` is not a feature.",
      },
      {
        value: "navigator.gpu.wgslLanguageFeatures",
        status: OK,
        since: "0.5.0",
        note: "The WGSL language extensions the shader compiler implements: `readonly_and_readwrite_storage_textures`, `packed_4x8_integer_dot_product` and `pointer_composite_access`.",
      },
      {
        value: "textureBindingViewDimension",
        status: OK,
        since: "0.5.0",
        note: "`undefined`: every device is a `core-features-and-limits` device.",
      },
      {
        value: "GPUTextureUsage.TRANSIENT_ATTACHMENT",
        status: OK,
        since: "0.5.0",
      },
      {
        value: "maxBindGroupsPlusVertexBuffers",
        status: OK,
        since: "0.5.0",
        note: "With `maxImmediateSize` and the per-stage `maxStorageBuffersInVertexStage`, `maxStorageBuffersInFragmentStage`, `maxStorageTexturesInVertexStage` and `maxStorageTexturesInFragmentStage`, which equal the per-shader-stage limits.",
      },
    ],
    limitations: [
      "No `importExternalTexture`, and `copyExternalImageToTexture` does not take a `<video>` or a `VideoFrame`.",
      "`adapter.features` lists only `core-features-and-limits` and `timestamp-query`. Optional features such as `shader-f16`, `subgroups` and the compressed texture formats are not offered.",
    ],
    related: ["offscreen-canvas", "webgl", "webgl-2"],
    aliases: [
      "navigator.gpu",
      "GPU",
      "GPUDevice",
      "getContext('webgpu')",
      "GPURenderBundle",
      "GPUQuerySet",
      "GPUPipelineError",
    ],
  }),
  api("webgl", "WebGL", GONE, "Renders 3D graphics on a canvas with an OpenGL ES 2.0 API.", {
    notes: ["`getContext('webgl')` returns `null`."],
    workaround: "Use WebGPU.",
    related: ["webgpu", "webgl-2"],
    aliases: ["WebGLRenderingContext", "getContext('webgl')"],
  }),
  api("webgl-2", "WebGL 2", GONE, "Renders 3D graphics on a canvas with an OpenGL ES 3.0 API.", {
    notes: ["`getContext('webgl')` returns `null`."],
    workaround: "Use WebGPU.",
    related: ["webgpu", "webgl"],
    aliases: ["WebGL2RenderingContext", "getContext('webgl2')"],
  }),
  api("web-audio", "Web Audio", PART, "Processes and synthesizes audio in a graph of nodes.", {
    notes: [
      "Multichannel graphs follow the spec: `channelCount`, `channelCountMode` and `channelInterpretation` on every node, with the speaker up- and down-mix rules for mono, stereo, quad and 5.1 and discrete mixing otherwise, up to 32 channels.",
      "`destination.maxChannelCount` is the output device's channel count. Raising `destination.channelCount` (up to it) plays each channel on its speaker, so a 5.1 or 7.1 graph plays as such on a 5.1 or 7.1 device; a narrower destination is mixed up onto the device's layout.",
      "`ChannelSplitterNode`, `ChannelMergerNode` and decoded `AudioBuffer`s keep their real channel counts.",
      "`DelayNode`, `WaveShaperNode`, `DynamicsCompressorNode`, `ConvolverNode` and `PeriodicWave` are there. Every node and `AudioBuffer` can be made by its constructor, `new GainNode(context, options)`, as well as by the factory on the context, and answers its `context`. `ConstantSourceNode`, `IIRFilterNode` and `OfflineAudioContext` are there too, and contexts and nodes are `EventTarget`s (`statechange`, `ended`, `complete`). A `ConvolverNode` takes mono, stereo and four-channel impulse responses and adds no latency.",
      "`suspend()` stops rendering and `currentTime`; `resume()` goes on from the same time.",
      "On a machine with no audio output a context runs silently, as in a browser: `currentTime` moves, sources end and analysers and worklets see the signal. It moves to a device when the system gets a default output. A device chosen with `setSinkId()` that is gone is still an error.",
    ],
    limitations: [
      "`OfflineAudioContext` renders in one piece: `suspend(time)` and `resume()` reject with `NotSupportedError`. Scheduled starts, stops and parameter changes land on the 128-frame block they fall in. It is an `instanceof AudioContext`.",
      "A `ConstantSourceNode` or any other node cannot be connected to an `AudioParam`. An `IIRFilterNode` filters at most 2 channels.",
      "A feedback loop needs a `DelayNode` in it, as in the spec, but the delay of a node in a loop is at least 1024 frames (about 21 ms at 48 kHz), not 128. Outside a loop any `delayTime` down to 0 is exact.",
      "`AudioParam` values are applied once per frame and smoothed, so `delayTime` glides to a new value and audio-rate modulation of a parameter is not possible.",
      "`DynamicsCompressorNode` follows the spec's parameters, look-ahead and make-up gain, but its knee and release curves are not Chromium's, so the same settings sound slightly different.",
      "A `PeriodicWave` keeps at most 1023 partials.",
      "`statechange` fires after the promise of `suspend()`, `resume()` or `close()` has resolved, and once after a context is made, which starts running.",
      "`fftSize` above 4096 throws a `TypeError`.",
      "A `BiquadFilterNode` filters at most 8 channels. `PannerNode`, `StereoPannerNode` and `MediaStreamAudioDestinationNode` take at most 2, and `channelCount` above that throws `NotSupportedError`.",
      "`createMediaElementSource()` returns a silent node: `<audio>` and `<video>` play through the platform's media engine, not the graph.",
    ],
    related: ["get-user-media", "spatial-audio", "audio-worklet"],
    aliases: ["AudioContext", "AudioNode", "channelCount", "maxChannelCount", "5.1", "7.1", "surround"],
  }),
  api(
    "audio-worklet",
    "AudioWorklet",
    PART,
    "Runs custom audio processing written in JavaScript inside a Web Audio graph.",
    {
      since: "0.7.0",
      notes: [
        "Experimental. `audioContext.audioWorklet.addModule(url)` loads a file of `AudioWorkletProcessor` classes, and `new AudioWorkletNode(context, name, options)` makes a node that runs one, with `parameters`, a `port` in both directions, `processorOptions` and the `processorerror` event. The [audio worklets guide](/guides/audio-worklets/) has an example and the Rust alternative.",
        "The processors of a context run on one thread with a JavaScript VM of its own. Its scope has the language, `AudioWorkletProcessor`, `registerProcessor`, `sampleRate`, `currentFrame`, `currentTime` and a `console`.",
        'Import the file with `?worklet`, or pass `new URL("./file.ts", import.meta.url)` to `addModule()`: the build bundles the file and what it imports into one script.',
      ],
      limitations: [
        "A node has at most one input and one output. The input has two channels, the output two or one (`outputChannelCount`).",
        "A node's output reaches the graph at least 256 frames later than in a browser, and later still after its processor has fallen behind.",
        "A parameter has one value for each block: `parameters.name` always has length 1.",
        "Messages and `processorOptions` are copied and never transferred: numbers, strings, booleans, `null`, `undefined`, arrays, plain objects, `ArrayBuffer`s and typed arrays. Anything else throws `DataCloneError`. A processor's port has `postMessage` and `onmessage` only; the node's port is an `EventTarget` named `AudioWorkletPort`.",
        "A worklet file has no `fetch`, timers or `import()`, and in an app built with `--aot` it is interpreted.",
        "`OfflineAudioContext` has no worklet: `addModule()` rejects with `NotSupportedError`.",
      ],
      related: ["web-audio"],
      aliases: ["AudioWorkletNode", "AudioWorkletProcessor", "registerProcessor", "addModule", "audioWorklet"],
    },
  ),
  api(
    "spatial-audio",
    "Spatial audio",
    PART,
    "Renders `PannerNode`s as Windows Sonic, Dolby Atmos or DTS audio objects.",
    {
      since: "0.4.0",
      skathiOnly: true,
      platforms: { windows: OK, macos: PLAN, linux: PLAN },
      notes: [
        "`panner.panningModel = 'spatial'` hands the panner to the system's spatial sound renderer (`ISpatialAudioClient`) as a dynamic audio object: the renderer gets its signal before panning, its position relative to the listener and its distance and cone gain, and does the 3D rendering itself.",
        "The rest of the graph plays in the renderer's static bed, up to 7.1.4, on the device `setSinkId()` chose.",
        "Dolby Atmos for Headphones ignores the position of dynamic objects, so on that format Skathi pans each `'spatial'` panner across the 7.1.4 bed itself, between the two nearest speakers of a layer and between ear level and the heights by elevation.",
        "`audioContext.spatialAudio` reports `{ active, requested, format, maxObjects, objectsInUse, reason }`. `format` names the renderer (`windows-sonic`, `dolby-atmos-headphones`, `dolby-atmos-home-theater`, `dolby-atmos-speakers`, `dts-headphone-x`, `dts-x-ultra`, `dts-x-home-theater`).",
        "A context switches to the spatial renderer when its first panner asks for `'spatial'`. Contexts without one play through the ordinary output, as in a browser.",
      ],
      limitations: [
        "Windows only, and only with a spatial sound format on for the device (Sound settings › Spatial sound). Without one, on macOS and Linux, and when the renderer fails, `'spatial'` panners render with HRTF and `spatialAudio.reason` says why; the reason is also logged once.",
        "The renderer has a limited number of objects (`spatialAudio.maxObjects`, often 15 to 32, shared with other apps; Skathi asks for at most 64). Panners past the limit, or whose object the renderer takes back, render with HRTF.",
        "An object leaves the graph at its panner. The `GainNode`s between the panner and the destination still scale it (the loudest path counts), and `suspend()` and a panner connected to no destination silence it, but other nodes after the panner (filters, analysers, stream destinations) do not see its sound. Put effects before the panner.",
        "A stereo input is mixed to mono for its object.",
        "Off with the `spatial-audio` Cargo feature off; `'spatial'` is then an invalid value and ignored.",
      ],
      related: ["web-audio"],
      aliases: [
        "panningModel",
        "spatialAudio",
        "Windows Sonic",
        "Dolby Atmos",
        "DTS",
        "ISpatialAudioClient",
        "audio objects",
        "3D audio",
      ],
    },
  ),
  method("getUserMedia", PART, "Asks for access to a camera or microphone and returns a media stream.", {
    notes: [MIC_NOTE],
    limitations: [MIC_LIMIT],
    related: CAPTURE.filter((s) => s !== "get-user-media"),
    aliases: ["navigator.mediaDevices.getUserMedia"],
  }),
  iface("MediaDevices", PART, "Access to the connected cameras and microphones.", {
    notes: [MIC_NOTE],
    limitations: [MIC_LIMIT],
    related: CAPTURE.filter((s) => s !== "media-devices"),
    aliases: ["navigator.mediaDevices", "enumerateDevices"],
  }),
  iface("MediaStream", PART, "A stream of audio or video tracks.", {
    notes: [MIC_NOTE],
    limitations: [MIC_LIMIT],
    related: CAPTURE.filter((s) => s !== "media-stream"),
    aliases: ["MediaStreamTrack"],
  }),
  el("video", "<video>", PART, "Embeds a video player in the document.", {
    notes: ["Plays."],
    limitations: ["`drawImage(video)` on a canvas draws nothing yet."],
    related: ["canvas-2d"],
    aliases: ["HTMLVideoElement"],
  }),
  el("sk-surface", "<sk-surface>", PART, "Shows frames that native code renders, composited like any other element.", {
    since: "0.4.0",
    skathiOnly: true,
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: [
      "The embedding program presents frames to the surface named by the `name` attribute: `skathi::surface(name)` in Rust, from any thread. The newest frame wins; the element picks it up on its next paint, and only while it is visible.",
      "On Windows a frame can be a D3D11 or D3D12 texture on the renderer's own device (copied on the GPU, after a GPU wait on the producer's fence for D3D12), or a shared texture from another device, API (D3D11, D3D12, Vulkan) or process, synchronised with a keyed mutex or a shared fence. On macOS it can be a Metal texture on the renderer's device (copied after a GPU wait on the producer's `MTLSharedEvent`), an IOSurface from another device or process, or a CVPixelBuffer (BGRA, or 4:2:0 converted on the GPU). CPU BGRA frames work on every platform.",
      "`object-fit` (default `contain`), `object-position`, `border-radius`, `overflow` clipping, transforms, opacity and z-order apply as on `<video>`. The box is 300 × 150 unless CSS sizes it.",
      "`surfaceresize` reports the painted content box in physical pixels (`width`, `height`, `devicePixelRatio`), so the producer can render at the size shown. `surfaceready` fires when the first frame shows, `surfacelost` when the frame goes (`reason`: `cleared` or `device`).",
      "`surfacedisplaychange` reports the screen the element is on: `headroom` and `potentialHeadroom` (1 is SDR white), `refreshRate`, `sdrWhiteNits`, `hdr` and `variableRefresh`, `null` where the platform does not say, and `outputHdr`, whether the window draws HDR now.",
    ],
    limitations: [
      "On Linux frames are CPU only: its GPU path (dmabuf) is not there yet, and `surfacedisplaychange` does not fire.",
      "Alpha is straight, not premultiplied. Frames are BGRA8, or half-float RGBA on a surface set to `Rgba16Float`: clipped at SDR white in a standard window, and up to the element's `dynamic-range-limit` in a window drawing HDR (Windows and macOS).",
    ],
    related: ["video", "canvas-2d"],
    aliases: [
      "native surface",
      "shared texture",
      "D3D11 texture",
      "D3D12 texture",
      "Metal texture",
      "IOSurface",
      "CVPixelBuffer",
      "Vulkan",
      "mpv",
      "external renderer",
    ],
  }),
  prop("navigator.mediaSession", OK, "Shows what the app is playing in the system's media controls.", {
    platforms: NOT_ON_LINUX,
    notes: [MEDIA_SESSION_NOTE],
    related: ["media-metadata"],
    aliases: ["MediaSession", "media keys", "Now Playing"],
  }),
  iface("MediaMetadata", OK, "The title, artist, album and artwork of the media being played.", {
    platforms: NOT_ON_LINUX,
    notes: [MEDIA_SESSION_NOTE],
    related: ["navigator-media-session"],
  }),
  iface("MediaRecorder", NO, "Records a media stream to a file.", {
    related: ["media-stream"],
  }),
  api("webcodecs", "WebCodecs", NO, "Encodes and decodes audio and video frames.", {
    aliases: ["VideoEncoder", "VideoDecoder", "AudioEncoder", "AudioDecoder"],
  }),
  api("webrtc", "WebRTC", NO, "Real-time peer-to-peer audio, video and data connections.", {
    aliases: ["RTCPeerConnection", "RTCDataChannel"],
  }),
]);

const LANGUAGE_NOTE = "The system's language preferences.";
const BADGE_NOTE = "Windows: a badge on the taskbar button; macOS: the Dock icon's badge.";
const SHARE_NOTE = "The system share sheet; text, URLs and files.";
const NOT_PLANNED_DEVICES = ["vibration", "web-bluetooth", "webusb", "webhid", "web-serial"];
const deviceRelated = (slug: string) => NOT_PLANNED_DEVICES.filter((s) => s !== slug);

const systemIntegration = section("System integration", [
  prop("navigator.language", OK, "The user's preferred language.", {
    notes: [LANGUAGE_NOTE],
    related: ["navigator-languages", "accept-language"],
  }),
  prop("navigator.languages", OK, "The user's preferred languages, most preferred first.", {
    notes: [LANGUAGE_NOTE],
    related: ["navigator-language", "accept-language"],
  }),
  prop("navigator.userAgentData", OK, "Structured information about the browser and platform.", {
    notes: ["`getHighEntropyValues` is supported."],
    aliases: ["NavigatorUAData", "getHighEntropyValues", "User-Agent Client Hints"],
  }),
  method("navigator.permissions.query", OK, "Reports the state of a permission.", {
    aliases: ["navigator.permissions", "Permissions API"],
  }),
  api("async-clipboard", "Async Clipboard", PART, "Reads and writes the system clipboard asynchronously.", {
    platforms: { windows: OK, macos: PART, linux: PART },
    limitations: [
      "Only Windows writes `ClipboardItem`s (`text/plain`, `text/html`, `image/png`). On macOS and Linux every `write()` rejects with `NotAllowedError` and `read()` returns only `text/plain`, while `writeText` and `readText` work.",
    ],
    aliases: ["navigator.clipboard", "ClipboardItem", "clipboard.writeText", "clipboard.readText"],
  }),
  iface("PressureObserver", PART, "Observes changes in CPU pressure for the visible application window.", {
    since: "0.7.0",
    aliases: ["Compute Pressure API", "PressureRecord", "PressureObserver.knownSources"],
    notes: [
      'Construct with a callback, then call `observe("cpu", { sampleInterval })`. Records contain `source`, `state` and a monotonic `time` in milliseconds. `takeRecords()` drains queued records. `unobserve("cpu")` and `disconnect()` stop observation.',
    ],
    limitations: [
      "Observers run in the application window. Worker observers and cross-origin permissions policies are outside this native window implementation.",
      "The only supported source is `cpu`. See [adaptive effects](/guides/adaptive-effects/) for the platform signals and examples.",
    ],
    related: ["navigator-get-battery"],
  }),
  method("navigator.getBattery", OK, "Reports the battery's charge level and charging state.", {
    aliases: ["BatteryManager", "Battery Status API"],
  }),
  prop("navigator.userActivation", OK, "Reports whether the user has interacted with the page.", {
    aliases: ["UserActivation", "user activation"],
  }),
  prop(
    "navigator.windowControlsOverlay",
    OK,
    "The area of the title bar that the window controls cover, for an app that draws its own title bar.",
    {
      aliases: ["WindowControlsOverlay", "titlebar-area"],
    },
  ),
  prop("launchQueue", OK, "Hands an installed app the files or URLs it was launched with.", {
    notes: ["Delivers the files and URLs the app was opened with."],
    aliases: ["LaunchParams", "window.launchQueue", "file handling"],
  }),
  method("navigator.setAppBadge", OK, "Shows a badge on the app's icon.", {
    platforms: NOT_ON_LINUX,
    notes: [BADGE_NOTE],
    related: ["navigator-clear-app-badge"],
    aliases: ["setAppBadge", "Badging API"],
  }),
  method("navigator.clearAppBadge", OK, "Removes the badge from the app's icon.", {
    platforms: NOT_ON_LINUX,
    notes: [BADGE_NOTE],
    related: ["navigator-set-app-badge"],
    aliases: ["clearAppBadge", "Badging API"],
  }),
  prop("navigator.wakeLock", OK, "Keeps the screen from dimming or locking.", {
    platforms: NOT_ON_LINUX,
    notes: ["`screen` wake locks are supported.", "Released when the window is minimized or closed."],
    aliases: ["WakeLock", "wakeLock.request", "Screen Wake Lock"],
  }),
  method("navigator.share", OK, "Shares text, URLs or files through the system's share mechanism.", {
    platforms: NOT_ON_LINUX,
    notes: [SHARE_NOTE],
    related: ["navigator-can-share"],
    aliases: ["Web Share"],
  }),
  method("navigator.canShare", OK, "Reports whether some data can be shared with `navigator.share`.", {
    platforms: NOT_ON_LINUX,
    notes: [SHARE_NOTE],
    related: ["navigator-share"],
    aliases: ["Web Share"],
  }),
  api("passkeys", "Passkeys", OK, "Signs in with public-key credentials instead of passwords.", {
    platforms: NOT_ON_LINUX,
    notes: [
      "Windows Hello, iCloud Keychain, security keys, phones.",
      "With the relying party named by the app, the client data's `origin` is `https://<rpId>`, which is what the relying party's server should expect.",
      "A page served over https from the relying party's domain keeps its own origin and needs neither `rp.id` nor `rpId`.",
    ],
    limitations: [
      "No conditional mediation (autofill).",
      "A web page's relying party ID defaults to its own domain. An app has no web origin, so it names the relying party itself: `rp.id` in `create()`, `rpId` in `get()`.",
      "On macOS the system serves a relying party only to an app associated with its domain: the app must be signed with a provisioning profile that grants the Associated Domains entitlement `webcredentials:<rpId>`, and `https://<rpId>/.well-known/apple-app-site-association` must list the app under `webcredentials`.",
      "On macOS, without that association, `create()` and `get()` reject with `NotAllowedError` and `isUserVerifyingPlatformAuthenticatorAvailable()` resolves `false`.",
      "On macOS the client data is the system's own, with the origin `https://<rpId>`.",
    ],
    aliases: [
      "navigator.credentials",
      "PublicKeyCredential",
      "WebAuthn",
      "credentials.create",
      "credentials.get",
      "rpId",
    ],
  }),
  iface("Notification", GONE, "Shows a system notification.", {
    aliases: ["Notifications API", "Notification.requestPermission"],
  }),
  api("gamepad", "Gamepad", NO, "Reads the state of connected game controllers.", {
    aliases: ["navigator.getGamepads", "getGamepads", "Gamepad API"],
  }),
  api("geolocation", "Geolocation", NO, "Reports the device's location.", {
    aliases: ["navigator.geolocation", "getCurrentPosition", "watchPosition"],
  }),
  api("vibration", "Vibration", NO, "Vibrates the device.", {
    related: deviceRelated("vibration"),
    aliases: ["navigator.vibrate"],
  }),
  api("web-bluetooth", "Web Bluetooth", NO, "Connects to Bluetooth Low Energy devices.", {
    related: deviceRelated("web-bluetooth"),
    aliases: ["navigator.bluetooth"],
  }),
  api("webusb", "WebUSB", NO, "Connects to USB devices.", {
    related: deviceRelated("webusb"),
    aliases: ["navigator.usb"],
  }),
  api("webhid", "WebHID", NO, "Connects to human interface devices such as keyboards and game controllers.", {
    related: deviceRelated("webhid"),
    aliases: ["navigator.hid"],
  }),
  api("web-serial", "Web Serial", NO, "Connects to serial ports.", {
    related: deviceRelated("web-serial"),
    aliases: ["navigator.serial"],
  }),
]);

export const entries: readonly WebEntry[] = [
  ...languageAndScheduling,
  ...domAndEvents,
  ...networking,
  ...streams,
  ...storage,
  ...graphics,
  ...systemIntegration,
];

export const entriesBySlug: ReadonlyMap<string, WebEntry> = new Map(entries.map((e) => [e.slug, e]));
