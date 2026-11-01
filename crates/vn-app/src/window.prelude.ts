// Windowing & chrome, the JS half — docs/WINDOWING.md tiers 1 and 2.
//
// Everything here is BROWSER-STANDARD (`document.title`, `screen`, `resizeTo`,
// `matchMedia`, the Fullscreen API) or STANDARDS-TRACK
// (`navigator.windowControlsOverlay`). The namespaced tier-3 extension —
// `appWindow` — is built on `globalThis.__VN_WINDOW__` below and re-exported
// from `packages/vue-native/src/runtime.ts`, so an app imports it from
// "vue-native" rather than finding it on the global.
//
// TODO(redjs): all of this belongs in `vendor/redjs/redjs/src/webapi/dom.rs`
// as real native accessors — same TODO the `history` stub carries. It lives in
// the prelude because this milestone deliberately touches no redjs (the
// submodule is already dirty in the worktree), and because a prelude accessor
// is observationally identical from JS.
//
// ---------------------------------------------------------------------------
// AUTHORED HERE, SHIPPED AS `window.prelude.js`
// ---------------------------------------------------------------------------
//
// The `.js` next to this file is GENERATED and checked in —
// `bun packages/vue-native/scripts/gen-preludes.ts` — because JavaScriptCore
// executes JavaScript and there is no bundler between `include_str!` and the
// engine. Edit this file, regenerate, commit both.
//
// This file has no imports (it cannot: `rt.eval` evaluates the artifact as a
// classic script), so its ambient declarations come from `tsconfig.json` in
// this directory, which pulls in `prelude-host.d.ts` beside it plus the
// runtime's own `engine.d.ts` and `protocol.d.ts`. `protocol.d.ts` is the
// interesting one: `VnAppWindow` is the surface `runtime.ts` re-exports, so the
// producer and the consumer of tier 3 now answer to ONE declaration.
//
// A note on `document!`, which appears a dozen times below. `engine.d.ts`
// declares `document` as possibly-undefined because the runtime's unit-test
// harness runs in a world that has none. This file only ever runs inside the
// host, after redjs has created it — `__vn_window_state()` on line one would
// already have thrown otherwise — so every access asserts rather than branching
// on a case that cannot happen here.
(() => {
  // Bun's emitter parses this file as a module and drops the directive
  // prologue, so this line does not survive erasure by itself: the generator
  // re-emits `"use strict";` at the top of the artifact, where it makes the
  // whole evaluated script strict — which is what the IIFE directive did. Kept
  // here because it is the source of truth for the fact that this code REQUIRES
  // strict mode: sloppy mode would turn the guarded `globalThis.innerWidth`
  // seeding below from a throw into a silent no-op.
  "use strict";

  const warned = new Set<string>();
  const warn = (key: string, msg: string) => {
    if (warned.has(key)) return;
    warned.add(key);
    console.warn(`[vue-native compat] ${msg}`);
    __vn_window_advise(msg);
  };

  // ---- the per-frame state snapshot ---------------------------------------
  // One read per frame, pushed by the host observer; `refresh()` is the
  // exact-read escape hatch for the one-frame lag geometry WRITES have (gpui's
  // bounds cache is refreshed from WM_MOVE/WM_SIZE, so a read immediately
  // after a write still sees the old value — which is also what CSSOM View
  // tells authors to expect from `moveTo`).
  //
  // The two annotations are where `JSON.parse`'s `any` stops: `S` is the only
  // variable in this file that a host snapshot flows into, and pinning it here
  // is what makes every `S.something` below a checked read rather than an
  // untyped one.
  let S: VnWindowState = JSON.parse(__vn_window_state());
  const refresh = (): VnWindowState => (S = JSON.parse(__vn_window_state()));

  const ro = (obj: object, name: string, get: () => unknown) =>
    Object.defineProperty(obj, name, { get, configurable: true, enumerable: true });

  // ---- document.title (§1.1) ----------------------------------------------
  // The getter applies HTML's normalisation — strip leading/trailing ASCII
  // whitespace, collapse interior runs to a single space — so
  // `document.title = "  a\n b  "` reads back "a b" exactly as in a browser.
  // A divergence here would be gratuitous.
  const normalizeTitle = (v: unknown) => String(v).replace(/[\t\n\f\r ]+/g, " ").replace(/^ | $/g, "");
  let titleSlot = normalizeTitle(S.title);
  Object.defineProperty(document, "title", {
    get: () => titleSlot,
    // `unknown`, not `string`: this is the accessor a browser gives `title`,
    // and a browser stringifies whatever it is handed.
    set(v: unknown) {
      titleSlot = normalizeTitle(v);
      // The RAW string goes to the OS: SetWindowTextW is not the place to
      // normalise, and the taskbar shows what was set.
      __vn_window_set_title(String(v));
    },
    configurable: true,
    enumerable: true,
  });

  // ---- geometry reads (§1.2) ----------------------------------------------
  // `innerWidth`/`innerHeight`/`devicePixelRatio` are owned by redjs (the host
  // pushes them through `redjs_dom_resize`); the four the platform never had
  // are defined here. All are CSS px.
  //
  // NOTE for layout code: `outerHeight - innerHeight` is small but NON-ZERO
  // even in the frameless window (roughly the invisible WS_THICKFRAME resize
  // border) and is NOT the titlebar height —
  // `navigator.windowControlsOverlay.getTitlebarAreaRect().height` is.
  ro(globalThis, "outerWidth", () => S.outer.width);
  ro(globalThis, "outerHeight", () => S.outer.height);
  ro(globalThis, "screenX", () => S.outer.x);
  ro(globalThis, "screenY", () => S.outer.y);
  ro(globalThis, "screenLeft", () => S.outer.x);
  ro(globalThis, "screenTop", () => S.outer.y);
  if (!(globalThis.innerWidth > 0)) {
    globalThis.innerWidth = S.innerWidth;
    globalThis.innerHeight = S.innerHeight;
    globalThis.devicePixelRatio = S.devicePixelRatio;
  }

  // ---- geometry writes (§1.3) ---------------------------------------------
  // ALL FOUR SHIP. CSSOM View gates them on "is this an auxiliary browsing
  // context created by script" — we are that context permanently, so the
  // restriction (which exists against popunders in shared browser UI) does not
  // apply. Electron agrees; a PWA cannot; Tauri gates it behind a capability.
  //
  // Never throws: a refused call is a silent no-op, as in every UA. Best-effort
  // and clamped to the display's working area, which is the spec's own
  // "optionally, restrain" latitude.
  const BOUNDS_X = 1, BOUNDS_Y = 2, BOUNDS_W = 4, BOUNDS_H = 8, BOUNDS_CONTENT = 16;
  const finite = (v: unknown) => (typeof v === "number" && isFinite(v) ? v : undefined);
  // `r` is whatever an app passed — `appWindow.setBounds()` is public API — so
  // the nullish half of the type is load-bearing, not defensive padding: it is
  // what makes the `r &&` guards below reachable code rather than dead ones.
  const setBounds = (r: Partial<VnWindowRect> | null | undefined, content: boolean) => {
    const x = finite(r && r.x), y = finite(r && r.y);
    const w = finite(r && r.width), h = finite(r && r.height);
    let mask = content ? BOUNDS_CONTENT : 0;
    if (x !== undefined) mask |= BOUNDS_X;
    if (y !== undefined) mask |= BOUNDS_Y;
    if (w !== undefined) mask |= BOUNDS_W;
    if (h !== undefined) mask |= BOUNDS_H;
    __vn_window_set_bounds(x ?? 0, y ?? 0, w ?? 0, h ?? 0, mask);
  };
  // `resizeTo` resizes the VIEWPORT; `moveTo` positions the WINDOW top-left.
  // The asymmetry is the spec's, not ours.
  globalThis.resizeTo = (w, h) => setBounds({ width: w, height: h }, true);
  globalThis.resizeBy = (dw, dh) =>
    setBounds({ width: S.innerWidth + (dw || 0), height: S.innerHeight + (dh || 0) }, true);
  globalThis.moveTo = (x, y) => setBounds({ x, y }, false);
  globalThis.moveBy = (dx, dy) =>
    setBounds({ x: S.outer.x + (dx || 0), y: S.outer.y + (dy || 0) }, false);

  // ---- close / focus / blur (§1.4) ----------------------------------------
  // `pagehide` fires once per close, whichever way it came: a WM_CLOSE the app
  // did not veto (the host observer below), or `close()` — the titlebar X a
  // frameless app draws. A browser fires it on every unload, and code that
  // leaves a call or saves a draft on it (LiveKit's leave on `pagehide`)
  // relies on that. `beforeunload` is not asked again here: `close()` is what
  // an app calls once it has decided, after a veto included (D-27).
  let pageHidden = false;
  const hidePage = () => {
    if (pageHidden) return;
    pageHidden = true;
    globalThis.dispatchEvent(new Event("pagehide"));
  };
  const closeWindow = () => {
    hidePage();
    __vn_window_verb("close");
  };
  globalThis.close = closeWindow;
  // HTML says the UA *may* bring the window to the front, and every real UA has
  // a policy — Chrome's, for a background tab, is precisely "flash the taskbar
  // button instead". This host forbids stealing focus outright, so `focus()`
  // requests attention unless the app opted in. That is MORE browser-shaped
  // than raw activation, not less.
  globalThis.focus = () => {
    const raise = S.focusPolicy === "raise";
    if (!raise) {
      warn(
        "focus",
        '`window.focus()` requested attention instead of raising the window — this host is ' +
          'configured not to steal focus. Set `window.focus: "raise"` in vn.config.json (or ' +
          "VN_WINDOW_FOCUS=raise) to allow raising.",
      );
    }
    __vn_window_activate(raise);
  };
  // The modern spec text is, in effect, *do nothing*. Matching means doing
  // nothing — but "nothing happened" is exactly the outcome a developer
  // misreads as a bug, so it is said out loud. Explicitly NOT minimize:
  // `blur() !== minimize()`, and conflating them would be a silent divergence.
  globalThis.blur = () =>
    warn(
      "blur",
      "`window.blur()` did nothing — and does nothing in any browser since it was removed as a " +
        "popunder vector. There is no web API to send a window behind others.",
    );
  document!.hasFocus = () => S.focused;

  // ---- screen (§1.5) ------------------------------------------------------
  // Exact values, not the fuzzed ones a browser reports: there is no untrusted
  // origin here and no fingerprinting threat model. A divergence in the
  // permissive direction (D-35).
  //
  // The two `as` casts here, and the four more like them below, are all the
  // same one: redjs has no `Screen`, no `MediaQueryList` and no
  // `WindowControlsOverlay` class, so each of these objects is an `EventTarget`
  // that this file then grows the interface onto — half by assignment, half by
  // `ro()` accessor, which no inference can see. The assertion names what the
  // object IS by the time the line after the last `ro()` has run.
  const orientation = new EventTarget() as VnScreenOrientation;
  ro(orientation, "type", () => (S.screen.width >= S.screen.height ? "landscape-primary" : "portrait-primary"));
  ro(orientation, "angle", () => 0);
  orientation.lock = () => {
    warn("orientation.lock", "`ScreenOrientation.lock()` rejects with NotSupportedError, as it does in every desktop browser.");
    const e = new Error("screen.orientation.lock() is not available on this device.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };
  orientation.unlock = () => {};
  const screenObj = new EventTarget() as VnScreen;
  ro(screenObj, "width", () => S.screen.width);
  ro(screenObj, "height", () => S.screen.height);
  ro(screenObj, "availWidth", () => S.screen.availWidth);
  ro(screenObj, "availHeight", () => S.screen.availHeight);
  ro(screenObj, "availLeft", () => S.screen.availLeft);
  ro(screenObj, "availTop", () => S.screen.availTop);
  // "Return 24 if unknown" is what the CSSOM View spec says; every modern
  // browser reports 24 unconditionally.
  ro(screenObj, "colorDepth", () => 24);
  ro(screenObj, "pixelDepth", () => 24);
  ro(screenObj, "isExtended", () => S.screen.isExtended);
  ro(screenObj, "orientation", () => orientation);
  globalThis.screen = screenObj;
  // Its entire value is placing OTHER windows, which is the multi-window
  // milestone. A rejected promise + an advisory beats a listener that can only
  // ever disappoint.
  globalThis.getScreenDetails = () => {
    warn(
      "getScreenDetails",
      "`getScreenDetails()` is not implemented (v1 is single-window). `screen.isExtended` reports " +
        "whether more than one display exists. See docs/WINDOWING.md §8.",
    );
    const e = new Error("getScreenDetails() is not supported.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };

  // ---- matchMedia (§1.6) --------------------------------------------------
  // redjs ships an MQL that is `matches: false` FOREVER while the CSS side has
  // a real evaluator — so `@media (max-width: 600px)` worked and
  // `matchMedia("(max-width: 600px)")` lied. This bridges it to the SAME
  // `vn-css` evaluator against the SAME env, so the two can never disagree.
  const liveQueries = new Set<VnMediaQueryList>();
  const evaluate = (q: string) => {
    const r: VnMatchMediaResult = JSON.parse(__vn_window_match_media(q));
    if (r.error) warn(`mq:${q}`, `matchMedia(${JSON.stringify(q)}) ${r.error}`);
    return !!r.matches;
  };
  globalThis.matchMedia = (query) => {
    const q = String(query);
    const mql = new EventTarget() as VnMediaQueryList;
    let current = evaluate(q);
    ro(mql, "media", () => q);
    ro(mql, "matches", () => current);
    mql.onchange = null;
    // The legacy aliases are what a lot of shipped library code still calls,
    // and they are still in the spec (deprecated, not removed).
    mql.addListener = (fn) => mql.addEventListener("change", fn);
    mql.removeListener = (fn) => mql.removeEventListener("change", fn);
    mql.__vnReevaluate = () => {
      const next = evaluate(q);
      if (next === current) return;
      current = next;
      const ev = new Event("change");
      // The spec's MediaQueryListEvent carries these; a `change` handler that
      // reads `e.matches` is extremely common.
      try {
        Object.defineProperty(ev, "matches", { value: next, configurable: true });
        Object.defineProperty(ev, "media", { value: q, configurable: true });
      } catch {}
      if (typeof mql.onchange === "function") mql.onchange.call(mql, ev);
      mql.dispatchEvent(ev);
    };
    liveQueries.add(mql);
    return mql;
  };

  // ---- Fullscreen API (§1.7) ----------------------------------------------
  // A faithful WINDOW-LEVEL subset: the window goes fullscreen and the element
  // is recorded, but there is no `::backdrop` and no UA stretch-to-viewport
  // rule, so a fullscreened <div> does not visually fill the screen on its own.
  // Said out loud (A-05) rather than left to surprise.
  let fsStack: VnFullscreenTarget[] = [];
  ro(document!, "fullscreenElement", () => fsStack[fsStack.length - 1] ?? null);
  ro(document!, "fullscreenEnabled", () => true);
  const fireFs = (name: string, target: VnFullscreenTarget | undefined) => {
    const ev = new Event(name, { bubbles: true, composed: true });
    if (target && typeof target.dispatchEvent === "function") target.dispatchEvent(ev);
    document!.dispatchEvent(new Event(name, { bubbles: true, composed: true }));
  };
  // A `function`, not an arrow, precisely so it can be installed on a prototype
  // and read its receiver — the `this` parameter is the type of that fact.
  const requestFullscreen = function (this: VnFullscreenTarget, options?: VnFullscreenOptions) {
    const el = this;
    if (options && options.keyboardLock)
      warn("fs.keyboardLock", "`keyboardLock` is ignored — there is no browser UI to lock.");
    if (options && options.screen)
      warn(
        "fs.screen",
        "the `screen` option needs the Window Management API, which is single-window-gated here. " +
          "See docs/WINDOWING.md §1.5.",
      );
    if (el !== document!.documentElement && el !== document!.body) {
      warn(
        "fs.element",
        "`requestFullscreen()` on an element made the WINDOW fullscreen; per-element fullscreen " +
          "(`::backdrop`, the UA stretch-to-viewport rule) is not implemented. Fullscreen " +
          "`document.documentElement`, or style the element under " +
          "`@media (display-mode: fullscreen)`.",
      );
    }
    fsStack.push(el);
    __vn_window_set_fullscreen(true);
    return Promise.resolve();
  };
  document!.exitFullscreen = () => {
    fsStack.pop();
    if (fsStack.length === 0) __vn_window_set_fullscreen(false);
    return Promise.resolve();
  };
  if (typeof Element === "function" && Element.prototype)
    Element.prototype.requestFullscreen = requestFullscreen;
  // `engine.d.ts` types `documentElement` as `unknown` — redjs's document is a
  // property bag with no tree behind it, and what sits there is whatever
  // `runtime.ts` installed. The cast is the claim this line already made in
  // JavaScript: whatever it is, it is a thing that can carry the method.
  if (document!.documentElement) (document!.documentElement as VnFullscreenTarget).requestFullscreen = requestFullscreen;

  // ---- windowControlsOverlay (§2.3) ---------------------------------------
  // The rect describes the area available to APP CONTENT. On macOS the traffic
  // lights sit LEFT, so `x` is non-zero there — correct code reads the rect and
  // never assumes a side (this also matters for RTL).
  //
  // redjs creates `navigator` lazily, so create-if-absent with the same guard.
  const nav = (globalThis.navigator ||= {});
  const wco = new EventTarget() as VnWindowControlsOverlay;
  ro(wco, "visible", () => S.titleBarOverlayVisible);
  wco.getTitlebarAreaRect = () => {
    const r = S.titleBarArea;
    // A plain object rather than a real DOMRect: redjs has no DOMRect class,
    // and every WCO example only ever reads x/y/width/height.
    return {
      x: r.x, y: r.y, width: r.width, height: r.height,
      top: r.y, left: r.x, right: r.x + r.width, bottom: r.y + r.height,
      toJSON() { return { ...r }; },
    };
  };
  wco.ongeometrychange = null;
  Object.defineProperty(nav, "windowControlsOverlay", { value: wco, configurable: true, enumerable: true });

  // ---- lifecycle (§1.8, §1.9) ---------------------------------------------
  // `unload` is being removed from the platform (Chrome finished in M153) and
  // loses ≥10% of beacon deliveries versus `pagehide`; not implemented.
  const addEL = globalThis.addEventListener?.bind(globalThis);
  if (addEL) {
    globalThis.addEventListener = (type, fn, opts) => {
      if (type === "unload")
        warn(
          "unload",
          "`unload` is being removed from the web platform (Chrome M153, Sept 2026) and is not " +
            "implemented. Use `pagehide` or `visibilitychange`.",
        );
      return addEL(type, fn, opts);
    };
  }
  // A-22. Still reserved, still an advisory, and deliberately NOT what
  // `<VnWindow>` consumes: `window.open` means "a second DOCUMENT" (a second
  // realm, a second `createApp`, `postMessage` as the only channel), which is a
  // different feature from "one tree, two windows" — D-M04. Shipping multi-
  // window must not pre-empt its semantics, so the message names the supported
  // path instead of pretending there is none.
  globalThis.open = () => {
    warn(
      "window.open",
      "`window.open()` is not supported: it means a SECOND DOCUMENT, and the reserved design " +
        "is in docs/WINDOWING.md §8. To paint part of THIS tree in a second OS window — a tool " +
        "palette, a detached player — use `<VnWindow>` from \"vue-native\" " +
        "(docs/MULTIWINDOW.md §1).",
    );
    return null;
  };

  // Sticky activation gates the `beforeunload` veto, exactly as Chrome gates
  // its dialog: an app that has never been touched must not be able to refuse
  // to close, which would be strictly worse behaviour than a web page.
  let sticky = false;
  const markSticky = () => (sticky = true);
  if (addEL) {
    addEL("keydown", markSticky, true);
    addEL("mousedown", markSticky, true);
  }
  globalThis.__vnStickyActivation = markSticky;

  // ---- appWindow (tier 3, §3) ---------------------------------------------
  // Every method is SYNCHRONOUS and returns undefined: JS, gpui and the Win32
  // UI thread are the same thread, so a Promise would lie about ordering.
  // `await undefined` is legal, so ported Tauri code (`await appWindow
  // .maximize()`) still runs.
  //
  // `VnAppWindow` comes from the RUNTIME's protocol.d.ts, not from a local
  // declaration: this object and `import { appWindow } from "vue-native"` are
  // the same object, so a verb that changes shape here must stop compiling
  // there. That is the one type in this file that is worth more than its cost.
  const appWindow = new EventTarget() as VnAppWindow;
  const verb = (name: string) => () => __vn_window_verb(name);
  appWindow.minimize = verb("minimize");
  appWindow.maximize = verb("maximize");
  appWindow.unmaximize = verb("unmaximize");
  appWindow.toggleMaximize = verb("toggleMaximize");
  appWindow.restore = verb("unmaximize");
  appWindow.close = closeWindow;
  appWindow.center = verb("center");
  appWindow.startDragging = verb("startDragging");
  appWindow.setFullscreen = (on) => __vn_window_set_fullscreen(!!on);
  appWindow.isMaximized = () => refresh().maximized;
  appWindow.isMinimized = () => refresh().minimized;
  appWindow.isFullscreen = () => refresh().fullscreen;
  appWindow.isFocused = () => refresh().focused;
  appWindow.getBounds = () => ({ ...S.outer });
  appWindow.getContentBounds = () => ({ ...S.content });
  appWindow.setBounds = (r) => setBounds(r, false);
  appWindow.setContentBounds = (r) => setBounds(r, true);
  // Electron's shape. `false` retires the cluster, an object installs it; the
  // host draws min/max/close in the top-right corner and gives each one a
  // `WindowControlArea`, so the OS performs the action and `HTMAXBUTTON` arms
  // the Windows 11 snap-layouts flyout. `getTitlebarAreaRect().width` loses the
  // cluster's width, and `geometrychange` fires.
  //
  // The two mode-3 cases are refused rather than faked: under
  // `titleBarStyle: "default"` the OS already drew real buttons.
  // -1 = "leave the cluster as it is, only the height moved"; 0 = off; 1 = on.
  //
  // `height` and the two colours are `unknown` because every caller reaches
  // here holding a value an app handed it; the `typeof` and the `String()` are
  // the coercion, and typing the parameters as what they OUGHT to be would only
  // move the lie one frame earlier.
  const setOverlay = (height: unknown, mode: number, color?: unknown, symbolColor?: unknown) => {
    __vn_window_set_overlay(
      typeof height === "number" ? height : NaN,
      mode,
      String(color || ""),
      String(symbolColor || ""),
    );
    refresh();
  };
  appWindow.setTitleBarOverlay = (o) => {
    if (o === false || o === null) return setOverlay(NaN, 0);
    if (o === true) return setOverlay(NaN, 1);
    if (!o || typeof o !== "object") return;
    // Electron's rule: passing an object means "I want the overlay". A caller
    // that only wants the strip taller uses `titleBarHeight`, which never
    // installs a cluster.
    setOverlay(o.height, 1, o.color, o.symbolColor);
  };
  appWindow.startResizeDragging = () =>
    warn(
      "startResizeDragging",
      "`startResizeDragging()` is not implemented — the window already has live resize borders on " +
        "all four edges.",
    );
  // `as const` so the loop variable is the five literal names rather than
  // `string`: without it `appWindow[name]` has no index signature to resolve
  // against, and with it a typo here is a build error instead of a fifth verb
  // that nothing ever calls.
  for (const name of ["setProgressBar", "setOverlayIcon", "setBadgeCount", "setSkipTaskbar", "setIcon"] as const) {
    appWindow[name] = () =>
      warn(
        `taskbar:${name}`,
        `\`${name}()\` is not implemented in v1 (Windows taskbar integration, no web equivalent).`,
      );
  }
  Object.defineProperty(appWindow, "title", {
    get: () => document!.title,
    set: (v: string) => (document!.title = v),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(appWindow, "backdrop", {
    get: () => S.backdrop,
    set: (v: unknown) => { __vn_window_set_backdrop(String(v)); refresh(); },
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(appWindow, "alwaysOnTop", {
    get: () => S.alwaysOnTop,
    set: (v: unknown) => { __vn_window_set_always_on_top(!!v); refresh(); },
    configurable: true,
    enumerable: true,
  });
  ro(appWindow, "scaleFactor", () => S.devicePixelRatio);
  // The chrome mode read, so an app can branch without a media query.
  ro(appWindow, "chrome", () => ({
    titleBarStyle: S.titleBarStyle,
    displayMode: S.displayMode,
    titleBarHeight: S.titleBarArea.height,
    fallbackTitleBar: S.fallbackTitleBar,
    overlayVisible: S.titleBarOverlayVisible,
    // Whether the HOST draws the min/max/close cluster (`titleBarOverlay`).
    hostControls: S.hostControls,
    // The window's client area, and how far the app's viewport is inset inside
    // it by host-drawn chrome. `inner* === client* - inset` by construction —
    // and in custom-chrome mode the inset is zero on all four sides.
    //
    // This is the metric the merge report's known issue asked for: before it,
    // `innerHeight` was the client area while every `getBoundingClientRect()`
    // and `documentElement.clientHeight` were root-relative, so anything that
    // sized itself against the viewport was optimistic by the chrome's height.
    client: { width: S.clientWidth, height: S.clientHeight },
    contentInset: { ...S.contentInset },
  }));

  // Creation-time-only keys: gpui fixes these when the window is created, so a
  // runtime setter would be a lie. Naming the file that CAN set them is the
  // whole value of the advisory (A-16).
  const CREATION_ONLY = [
    "width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight",
    "resizable", "minimizable", "maximizable", "closable", "movable",
    "titleBarFallback", "focus",
  ];
  // These two are not merely creation-time: they need a different WINDOW STYLE,
  // and `SetWindowLongPtr(GWL_STYLE)` + SWP_FRAMECHANGED mid-session desyncs
  // gpui's cached `border_offset` and hit-test flags — and flickers. Refused
  // rather than faked (D-25), with its own wording.
  const STYLE_ONLY = ["frame", "titleBarStyle"];
  // Each of the four `as` casts below is the same admission: `opts` is a bag an
  // app wrote, every value in it is `unknown`, and the property being assigned
  // is typed as what the SETTER accepts after coercion — `document.title`'s
  // setter `String()`s, `appWindow.backdrop`'s does too, `alwaysOnTop`'s takes
  // `!!v`. The cast asserts nothing the setter does not already handle; what it
  // buys is that the KEY has to be one this file knows about.
  const defineWindow = (opts: Record<string, unknown> | null | undefined) => {
    if (!opts || typeof opts !== "object") return;
    for (const key of Object.keys(opts)) {
      if (key === "title") { document!.title = opts.title as string; continue; }
      if (key === "backdrop") { appWindow.backdrop = opts.backdrop as string; continue; }
      if (key === "alwaysOnTop") { appWindow.alwaysOnTop = opts.alwaysOnTop as boolean; continue; }
      if (key === "titleBarHeight") { setOverlay(opts.titleBarHeight, -1); continue; }
      if (key === "titleBarOverlay") { appWindow.setTitleBarOverlay(opts.titleBarOverlay as boolean); continue; }
      if (STYLE_ONLY.includes(key)) {
        warn(
          `style-only:${key}`,
          `\`${key}\` cannot change after the window is created — it needs a different window ` +
            "style, which would desync the hit test and flicker. Declare it in vn.config.json.",
        );
        continue;
      }
      if (CREATION_ONLY.includes(key)) {
        warn(
          `creation-only:${key}`,
          `\`${key}\` can only be set when the window is created. Declare it in ` +
            "`apps/<app>/vn.config.json` under `window`. (gpui fixes this at creation; see " +
            "docs/WINDOWING.md §9.)",
        );
        continue;
      }
      warn(`defineWindow:${key}`, `unknown \`defineWindow\` key: ${JSON.stringify(key)}`);
    }
  };

  // ---- the host observer --------------------------------------------------
  // ONE callback, called with a comma-separated kind list and the new snapshot.
  // Its boolean return is the `beforeunload` veto.
  let closeRequestedListeners = 0;
  const origAdd = appWindow.addEventListener.bind(appWindow);
  appWindow.addEventListener = (type, fn, opts) => {
    if (type === "close-requested") closeRequestedListeners += 1;
    return origAdd(type, fn, opts);
  };

  const emit = (target: VnDispatchTarget, name: string, init?: VnEventInit) =>
    target.dispatchEvent(new Event(name, init));

  __vn_window_observe((kinds, json) => {
    S = JSON.parse(json);
    // "sync" publishes the new snapshot and fires nothing: the host calls it
    // before the trusted `resize`, so every getter already answers with this
    // frame's numbers by the time the first handler runs.
    if (kinds === "sync") return true;
    if (kinds === "close-requested") {
      // A cancelled `beforeunload` vetoes the close and hands the app
      // `close-requested`; v1 shows no UA confirmation dialog (D-27).
      const ev = new Event("beforeunload", { cancelable: true });
      globalThis.dispatchEvent(ev);
      const vetoed = sticky && ev.defaultPrevented;
      if (!vetoed) {
        hidePage();
        return true;
      }
      if (closeRequestedListeners === 0) {
        warn(
          "beforeunload-trap",
          "`beforeunload` cancelled the close, but vue-native shows no confirmation dialog (v1) " +
            "and nothing is listening for `close-requested`. THIS WINDOW CAN NO LONGER BE CLOSED. " +
            "Listen for `close-requested` on `appWindow` and call `window.close()` when the user " +
            "confirms.",
        );
      }
      emit(appWindow, "close-requested", { cancelable: true });
      return false;
    }
    for (const kind of kinds.split(",")) {
      switch (kind) {
        case "boot":
        case "resize":
          // The trusted `resize` was already dispatched by the host through
          // redjs; nothing to do but re-evaluate the live media queries.
          break;
        case "focus":
        case "blur":
          emit(globalThis, kind);
          break;
        case "fullscreenchange":
          fireFs("fullscreenchange", fsStack[fsStack.length - 1]);
          if (!S.fullscreen) fsStack = [];
          break;
        case "geometrychange": {
          const ev = new Event("geometrychange");
          if (typeof wco.ongeometrychange === "function") wco.ongeometrychange.call(wco, ev);
          wco.dispatchEvent(ev);
          break;
        }
        case "mediachange":
          break;
        default:
          // maximize / unmaximize / minimize / restore / move
          emit(appWindow, kind);
          break;
      }
    }
    for (const mql of liveQueries) mql.__vnReevaluate();
    return true;
  });

  // The three return annotations are the second place `JSON.parse`'s `any`
  // stops (`S` was the first): without them the two debug hooks would hand a
  // caller `any` and every field read off a hitbox would be unchecked.
  globalThis.__VN_WINDOW__ = {
    appWindow,
    defineWindow,
    getCurrentWindow: () => appWindow,
    refresh,
    warn,
    // The eval-reachable debug hook for drag regions: `WM_NCHITTEST` is a real
    // OS message, so a synthesised click can never exercise one. This reports
    // what the last painted frame actually handed gpui.
    controlHitboxes: (windowId?: number): VnWindowHitbox[] =>
      JSON.parse(__vn_window_hitboxes(windowId)),
    cursorPos: (): VnWindowCursorPos => JSON.parse(__vn_window_cursor()),
    state: () => S,
  };
})();
