// Windowing & chrome, the JS half — docs/WINDOWING.md tiers 1 and 2.
//
// Everything here is BROWSER-STANDARD (`document.title`, `screen`, `resizeTo`,
// `matchMedia`, the Fullscreen API) or STANDARDS-TRACK
// (`navigator.windowControlsOverlay`). The namespaced tier-3 extension —
// `appWindow` — is built on `globalThis.__VN_WINDOW__` below and re-exported
// from `packages/vue-native/src/runtime.js`, so an app imports it from
// "vue-native" rather than finding it on the global.
//
// TODO(redjs): all of this belongs in `vendor/redjs/redjs/src/webapi/dom.rs`
// as real native accessors — same TODO the `history` stub carries. It lives in
// the prelude because this milestone deliberately touches no redjs (the
// submodule is already dirty in the worktree), and because a prelude accessor
// is observationally identical from JS.
(() => {
  "use strict";

  const warned = new Set();
  const warn = (key, msg) => {
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
  let S = JSON.parse(__vn_window_state());
  const refresh = () => (S = JSON.parse(__vn_window_state()));

  const ro = (obj, name, get) =>
    Object.defineProperty(obj, name, { get, configurable: true, enumerable: true });

  // ---- document.title (§1.1) ----------------------------------------------
  // The getter applies HTML's normalisation — strip leading/trailing ASCII
  // whitespace, collapse interior runs to a single space — so
  // `document.title = "  a\n b  "` reads back "a b" exactly as in a browser.
  // A divergence here would be gratuitous.
  const normalizeTitle = (v) => String(v).replace(/[\t\n\f\r ]+/g, " ").replace(/^ | $/g, "");
  let titleSlot = normalizeTitle(S.title);
  Object.defineProperty(document, "title", {
    get: () => titleSlot,
    set(v) {
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
  const finite = (v) => (typeof v === "number" && isFinite(v) ? v : undefined);
  const setBounds = (r, content) => {
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
  globalThis.close = () => __vn_window_verb("close");
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
  document.hasFocus = () => S.focused;

  // ---- screen (§1.5) ------------------------------------------------------
  // Exact values, not the fuzzed ones a browser reports: there is no untrusted
  // origin here and no fingerprinting threat model. A divergence in the
  // permissive direction (D-35).
  const orientation = new EventTarget();
  ro(orientation, "type", () => (S.screen.width >= S.screen.height ? "landscape-primary" : "portrait-primary"));
  ro(orientation, "angle", () => 0);
  orientation.lock = () => {
    warn("orientation.lock", "`ScreenOrientation.lock()` rejects with NotSupportedError, as it does in every desktop browser.");
    const e = new Error("screen.orientation.lock() is not available on this device.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };
  orientation.unlock = () => {};
  const screenObj = new EventTarget();
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
  const liveQueries = new Set();
  const evaluate = (q) => {
    const r = JSON.parse(__vn_window_match_media(q));
    if (r.error) warn(`mq:${q}`, `matchMedia(${JSON.stringify(q)}) ${r.error}`);
    return !!r.matches;
  };
  globalThis.matchMedia = (query) => {
    const q = String(query);
    const mql = new EventTarget();
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
  let fsStack = [];
  ro(document, "fullscreenElement", () => fsStack[fsStack.length - 1] ?? null);
  ro(document, "fullscreenEnabled", () => true);
  const fireFs = (name, target) => {
    const ev = new Event(name, { bubbles: true, composed: true });
    if (target && typeof target.dispatchEvent === "function") target.dispatchEvent(ev);
    document.dispatchEvent(new Event(name, { bubbles: true, composed: true }));
  };
  const requestFullscreen = function (options) {
    const el = this;
    if (options && options.keyboardLock)
      warn("fs.keyboardLock", "`keyboardLock` is ignored — there is no browser UI to lock.");
    if (options && options.screen)
      warn(
        "fs.screen",
        "the `screen` option needs the Window Management API, which is single-window-gated here. " +
          "See docs/WINDOWING.md §1.5.",
      );
    if (el !== document.documentElement && el !== document.body) {
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
  document.exitFullscreen = () => {
    fsStack.pop();
    if (fsStack.length === 0) __vn_window_set_fullscreen(false);
    return Promise.resolve();
  };
  if (typeof Element === "function" && Element.prototype)
    Element.prototype.requestFullscreen = requestFullscreen;
  if (document.documentElement) document.documentElement.requestFullscreen = requestFullscreen;

  // ---- windowControlsOverlay (§2.3) ---------------------------------------
  // The rect describes the area available to APP CONTENT. On macOS the traffic
  // lights sit LEFT, so `x` is non-zero there — correct code reads the rect and
  // never assumes a side (this also matters for RTL).
  //
  // redjs creates `navigator` lazily, so create-if-absent with the same guard.
  const nav = (globalThis.navigator ||= {});
  const wco = new EventTarget();
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
  globalThis.open = () => {
    warn(
      "window.open",
      "`window.open()` is not supported (v1 is single-window). The reserved design is in " +
        "docs/WINDOWING.md §8.",
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
  const appWindow = new EventTarget();
  const verb = (name) => () => __vn_window_verb(name);
  appWindow.minimize = verb("minimize");
  appWindow.maximize = verb("maximize");
  appWindow.unmaximize = verb("unmaximize");
  appWindow.toggleMaximize = verb("toggleMaximize");
  appWindow.restore = verb("unmaximize");
  appWindow.close = verb("close");
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
  const setOverlay = (height, mode, color, symbolColor) => {
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
  for (const name of ["setProgressBar", "setOverlayIcon", "setBadgeCount", "setSkipTaskbar", "setIcon"]) {
    appWindow[name] = () =>
      warn(
        `taskbar:${name}`,
        `\`${name}()\` is not implemented in v1 (Windows taskbar integration, no web equivalent).`,
      );
  }
  Object.defineProperty(appWindow, "title", {
    get: () => document.title,
    set: (v) => (document.title = v),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(appWindow, "backdrop", {
    get: () => S.backdrop,
    set: (v) => { __vn_window_set_backdrop(String(v)); refresh(); },
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(appWindow, "alwaysOnTop", {
    get: () => S.alwaysOnTop,
    set: (v) => { __vn_window_set_always_on_top(!!v); refresh(); },
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
  const defineWindow = (opts) => {
    if (!opts || typeof opts !== "object") return;
    for (const key of Object.keys(opts)) {
      if (key === "title") { document.title = opts.title; continue; }
      if (key === "backdrop") { appWindow.backdrop = opts.backdrop; continue; }
      if (key === "alwaysOnTop") { appWindow.alwaysOnTop = opts.alwaysOnTop; continue; }
      if (key === "titleBarHeight") { setOverlay(opts.titleBarHeight, -1); continue; }
      if (key === "titleBarOverlay") { appWindow.setTitleBarOverlay(opts.titleBarOverlay); continue; }
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

  const emit = (target, name, init) => target.dispatchEvent(new Event(name, init));

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
        emit(globalThis, "pagehide");
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

  globalThis.__VN_WINDOW__ = {
    appWindow,
    defineWindow,
    getCurrentWindow: () => appWindow,
    refresh,
    warn,
    // The eval-reachable debug hook for drag regions: `WM_NCHITTEST` is a real
    // OS message, so a synthesised click can never exercise one. This reports
    // what the last painted frame actually handed gpui.
    controlHitboxes: () => JSON.parse(__vn_window_hitboxes()),
    cursorPos: () => JSON.parse(__vn_window_cursor()),
    state: () => S,
  };
})();
