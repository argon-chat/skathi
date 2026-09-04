// GENERATED from crates/vn-app/src/window.prelude.ts — DO NOT EDIT.
//
//   source:   crates/vn-app/src/window.prelude.ts
//   emitter:  Bun.Transpiler (the same one build.ts uses for dist/vendor.js)
//   regen:    bun packages/vue-native/scripts/gen-preludes.ts
//   check:    bun packages/vue-native/scripts/gen-preludes.ts --check
//
// evaluated as `vn://window.js`, after the prelude and before the app bundle.
//
// Every comment in the source is gone: bun re-prints from the AST. Read the
// .ts — it is the file with the reasoning in it, and the only one an edit
// survives.
"use strict";
(() => {
  const warned = new Set;
  const warn = (key, msg) => {
    if (warned.has(key))
      return;
    warned.add(key);
    console.warn(`[vue-native compat] ${msg}`);
    __vn_window_advise(msg);
  };
  let S = JSON.parse(__vn_window_state());
  const refresh = () => S = JSON.parse(__vn_window_state());
  const ro = (obj, name, get) => Object.defineProperty(obj, name, { get, configurable: true, enumerable: true });
  const normalizeTitle = (v) => String(v).replace(/[\t\n\f\r ]+/g, " ").replace(/^ | $/g, "");
  let titleSlot = normalizeTitle(S.title);
  Object.defineProperty(document, "title", {
    get: () => titleSlot,
    set(v) {
      titleSlot = normalizeTitle(v);
      __vn_window_set_title(String(v));
    },
    configurable: true,
    enumerable: true
  });
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
  const BOUNDS_X = 1, BOUNDS_Y = 2, BOUNDS_W = 4, BOUNDS_H = 8, BOUNDS_CONTENT = 16;
  const finite = (v) => typeof v === "number" && isFinite(v) ? v : undefined;
  const setBounds = (r, content) => {
    const x = finite(r && r.x), y = finite(r && r.y);
    const w = finite(r && r.width), h = finite(r && r.height);
    let mask = content ? BOUNDS_CONTENT : 0;
    if (x !== undefined)
      mask |= BOUNDS_X;
    if (y !== undefined)
      mask |= BOUNDS_Y;
    if (w !== undefined)
      mask |= BOUNDS_W;
    if (h !== undefined)
      mask |= BOUNDS_H;
    __vn_window_set_bounds(x ?? 0, y ?? 0, w ?? 0, h ?? 0, mask);
  };
  globalThis.resizeTo = (w, h) => setBounds({ width: w, height: h }, true);
  globalThis.resizeBy = (dw, dh) => setBounds({ width: S.innerWidth + (dw || 0), height: S.innerHeight + (dh || 0) }, true);
  globalThis.moveTo = (x, y) => setBounds({ x, y }, false);
  globalThis.moveBy = (dx, dy) => setBounds({ x: S.outer.x + (dx || 0), y: S.outer.y + (dy || 0) }, false);
  globalThis.close = () => __vn_window_verb("close");
  globalThis.focus = () => {
    const raise = S.focusPolicy === "raise";
    if (!raise) {
      warn("focus", "`window.focus()` requested attention instead of raising the window — this host is " + 'configured not to steal focus. Set `window.focus: "raise"` in vn.config.json (or ' + "VN_WINDOW_FOCUS=raise) to allow raising.");
    }
    __vn_window_activate(raise);
  };
  globalThis.blur = () => warn("blur", "`window.blur()` did nothing — and does nothing in any browser since it was removed as a " + "popunder vector. There is no web API to send a window behind others.");
  document.hasFocus = () => S.focused;
  const orientation = new EventTarget;
  ro(orientation, "type", () => S.screen.width >= S.screen.height ? "landscape-primary" : "portrait-primary");
  ro(orientation, "angle", () => 0);
  orientation.lock = () => {
    warn("orientation.lock", "`ScreenOrientation.lock()` rejects with NotSupportedError, as it does in every desktop browser.");
    const e = new Error("screen.orientation.lock() is not available on this device.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };
  orientation.unlock = () => {};
  const screenObj = new EventTarget;
  ro(screenObj, "width", () => S.screen.width);
  ro(screenObj, "height", () => S.screen.height);
  ro(screenObj, "availWidth", () => S.screen.availWidth);
  ro(screenObj, "availHeight", () => S.screen.availHeight);
  ro(screenObj, "availLeft", () => S.screen.availLeft);
  ro(screenObj, "availTop", () => S.screen.availTop);
  ro(screenObj, "colorDepth", () => 24);
  ro(screenObj, "pixelDepth", () => 24);
  ro(screenObj, "isExtended", () => S.screen.isExtended);
  ro(screenObj, "orientation", () => orientation);
  globalThis.screen = screenObj;
  globalThis.getScreenDetails = () => {
    warn("getScreenDetails", "`getScreenDetails()` is not implemented (v1 is single-window). `screen.isExtended` reports " + "whether more than one display exists. See docs/WINDOWING.md §8.");
    const e = new Error("getScreenDetails() is not supported.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };
  const liveQueries = new Set;
  const evaluate = (q) => {
    const r = JSON.parse(__vn_window_match_media(q));
    if (r.error)
      warn(`mq:${q}`, `matchMedia(${JSON.stringify(q)}) ${r.error}`);
    return !!r.matches;
  };
  globalThis.matchMedia = (query) => {
    const q = String(query);
    const mql = new EventTarget;
    let current = evaluate(q);
    ro(mql, "media", () => q);
    ro(mql, "matches", () => current);
    mql.onchange = null;
    mql.addListener = (fn) => mql.addEventListener("change", fn);
    mql.removeListener = (fn) => mql.removeEventListener("change", fn);
    mql.__vnReevaluate = () => {
      const next = evaluate(q);
      if (next === current)
        return;
      current = next;
      const ev = new Event("change");
      try {
        Object.defineProperty(ev, "matches", { value: next, configurable: true });
        Object.defineProperty(ev, "media", { value: q, configurable: true });
      } catch {}
      if (typeof mql.onchange === "function")
        mql.onchange.call(mql, ev);
      mql.dispatchEvent(ev);
    };
    liveQueries.add(mql);
    return mql;
  };
  let fsStack = [];
  ro(document, "fullscreenElement", () => fsStack[fsStack.length - 1] ?? null);
  ro(document, "fullscreenEnabled", () => true);
  const fireFs = (name, target) => {
    const ev = new Event(name, { bubbles: true, composed: true });
    if (target && typeof target.dispatchEvent === "function")
      target.dispatchEvent(ev);
    document.dispatchEvent(new Event(name, { bubbles: true, composed: true }));
  };
  const requestFullscreen = function(options) {
    const el = this;
    if (options && options.keyboardLock)
      warn("fs.keyboardLock", "`keyboardLock` is ignored — there is no browser UI to lock.");
    if (options && options.screen)
      warn("fs.screen", "the `screen` option needs the Window Management API, which is single-window-gated here. " + "See docs/WINDOWING.md §1.5.");
    if (el !== document.documentElement && el !== document.body) {
      warn("fs.element", "`requestFullscreen()` on an element made the WINDOW fullscreen; per-element fullscreen " + "(`::backdrop`, the UA stretch-to-viewport rule) is not implemented. Fullscreen " + "`document.documentElement`, or style the element under " + "`@media (display-mode: fullscreen)`.");
    }
    fsStack.push(el);
    __vn_window_set_fullscreen(true);
    return Promise.resolve();
  };
  document.exitFullscreen = () => {
    fsStack.pop();
    if (fsStack.length === 0)
      __vn_window_set_fullscreen(false);
    return Promise.resolve();
  };
  if (typeof Element === "function" && Element.prototype)
    Element.prototype.requestFullscreen = requestFullscreen;
  if (document.documentElement)
    document.documentElement.requestFullscreen = requestFullscreen;
  const nav = globalThis.navigator ||= {};
  const wco = new EventTarget;
  ro(wco, "visible", () => S.titleBarOverlayVisible);
  wco.getTitlebarAreaRect = () => {
    const r = S.titleBarArea;
    return {
      x: r.x,
      y: r.y,
      width: r.width,
      height: r.height,
      top: r.y,
      left: r.x,
      right: r.x + r.width,
      bottom: r.y + r.height,
      toJSON() {
        return { ...r };
      }
    };
  };
  wco.ongeometrychange = null;
  Object.defineProperty(nav, "windowControlsOverlay", { value: wco, configurable: true, enumerable: true });
  const addEL = globalThis.addEventListener?.bind(globalThis);
  if (addEL) {
    globalThis.addEventListener = (type, fn, opts) => {
      if (type === "unload")
        warn("unload", "`unload` is being removed from the web platform (Chrome M153, Sept 2026) and is not " + "implemented. Use `pagehide` or `visibilitychange`.");
      return addEL(type, fn, opts);
    };
  }
  globalThis.open = () => {
    warn("window.open", "`window.open()` is not supported: it means a SECOND DOCUMENT, and the reserved design " + "is in docs/WINDOWING.md §8. To paint part of THIS tree in a second OS window — a tool " + 'palette, a detached player — use `<VnWindow>` from "vue-native" ' + "(docs/MULTIWINDOW.md §1).");
    return null;
  };
  let sticky = false;
  const markSticky = () => sticky = true;
  if (addEL) {
    addEL("keydown", markSticky, true);
    addEL("mousedown", markSticky, true);
  }
  globalThis.__vnStickyActivation = markSticky;
  const appWindow = new EventTarget;
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
  const setOverlay = (height, mode, color, symbolColor) => {
    __vn_window_set_overlay(typeof height === "number" ? height : NaN, mode, String(color || ""), String(symbolColor || ""));
    refresh();
  };
  appWindow.setTitleBarOverlay = (o) => {
    if (o === false || o === null)
      return setOverlay(NaN, 0);
    if (o === true)
      return setOverlay(NaN, 1);
    if (!o || typeof o !== "object")
      return;
    setOverlay(o.height, 1, o.color, o.symbolColor);
  };
  appWindow.startResizeDragging = () => warn("startResizeDragging", "`startResizeDragging()` is not implemented — the window already has live resize borders on " + "all four edges.");
  for (const name of ["setProgressBar", "setOverlayIcon", "setBadgeCount", "setSkipTaskbar", "setIcon"]) {
    appWindow[name] = () => warn(`taskbar:${name}`, `\`${name}()\` is not implemented in v1 (Windows taskbar integration, no web equivalent).`);
  }
  Object.defineProperty(appWindow, "title", {
    get: () => document.title,
    set: (v) => document.title = v,
    configurable: true,
    enumerable: true
  });
  Object.defineProperty(appWindow, "backdrop", {
    get: () => S.backdrop,
    set: (v) => {
      __vn_window_set_backdrop(String(v));
      refresh();
    },
    configurable: true,
    enumerable: true
  });
  Object.defineProperty(appWindow, "alwaysOnTop", {
    get: () => S.alwaysOnTop,
    set: (v) => {
      __vn_window_set_always_on_top(!!v);
      refresh();
    },
    configurable: true,
    enumerable: true
  });
  ro(appWindow, "scaleFactor", () => S.devicePixelRatio);
  ro(appWindow, "chrome", () => ({
    titleBarStyle: S.titleBarStyle,
    displayMode: S.displayMode,
    titleBarHeight: S.titleBarArea.height,
    fallbackTitleBar: S.fallbackTitleBar,
    overlayVisible: S.titleBarOverlayVisible,
    hostControls: S.hostControls,
    client: { width: S.clientWidth, height: S.clientHeight },
    contentInset: { ...S.contentInset }
  }));
  const CREATION_ONLY = [
    "width",
    "height",
    "minWidth",
    "minHeight",
    "maxWidth",
    "maxHeight",
    "resizable",
    "minimizable",
    "maximizable",
    "closable",
    "movable",
    "titleBarFallback",
    "focus"
  ];
  const STYLE_ONLY = ["frame", "titleBarStyle"];
  const defineWindow = (opts) => {
    if (!opts || typeof opts !== "object")
      return;
    for (const key of Object.keys(opts)) {
      if (key === "title") {
        document.title = opts.title;
        continue;
      }
      if (key === "backdrop") {
        appWindow.backdrop = opts.backdrop;
        continue;
      }
      if (key === "alwaysOnTop") {
        appWindow.alwaysOnTop = opts.alwaysOnTop;
        continue;
      }
      if (key === "titleBarHeight") {
        setOverlay(opts.titleBarHeight, -1);
        continue;
      }
      if (key === "titleBarOverlay") {
        appWindow.setTitleBarOverlay(opts.titleBarOverlay);
        continue;
      }
      if (STYLE_ONLY.includes(key)) {
        warn(`style-only:${key}`, `\`${key}\` cannot change after the window is created — it needs a different window ` + "style, which would desync the hit test and flicker. Declare it in vn.config.json.");
        continue;
      }
      if (CREATION_ONLY.includes(key)) {
        warn(`creation-only:${key}`, `\`${key}\` can only be set when the window is created. Declare it in ` + "`apps/<app>/vn.config.json` under `window`. (gpui fixes this at creation; see " + "docs/WINDOWING.md §9.)");
        continue;
      }
      warn(`defineWindow:${key}`, `unknown \`defineWindow\` key: ${JSON.stringify(key)}`);
    }
  };
  let closeRequestedListeners = 0;
  const origAdd = appWindow.addEventListener.bind(appWindow);
  appWindow.addEventListener = (type, fn, opts) => {
    if (type === "close-requested")
      closeRequestedListeners += 1;
    return origAdd(type, fn, opts);
  };
  const emit = (target, name, init) => target.dispatchEvent(new Event(name, init));
  __vn_window_observe((kinds, json) => {
    S = JSON.parse(json);
    if (kinds === "sync")
      return true;
    if (kinds === "close-requested") {
      const ev = new Event("beforeunload", { cancelable: true });
      globalThis.dispatchEvent(ev);
      const vetoed = sticky && ev.defaultPrevented;
      if (!vetoed) {
        emit(globalThis, "pagehide");
        return true;
      }
      if (closeRequestedListeners === 0) {
        warn("beforeunload-trap", "`beforeunload` cancelled the close, but vue-native shows no confirmation dialog (v1) " + "and nothing is listening for `close-requested`. THIS WINDOW CAN NO LONGER BE CLOSED. " + "Listen for `close-requested` on `appWindow` and call `window.close()` when the user " + "confirms.");
      }
      emit(appWindow, "close-requested", { cancelable: true });
      return false;
    }
    for (const kind of kinds.split(",")) {
      switch (kind) {
        case "boot":
        case "resize":
          break;
        case "focus":
        case "blur":
          emit(globalThis, kind);
          break;
        case "fullscreenchange":
          fireFs("fullscreenchange", fsStack[fsStack.length - 1]);
          if (!S.fullscreen)
            fsStack = [];
          break;
        case "geometrychange": {
          const ev = new Event("geometrychange");
          if (typeof wco.ongeometrychange === "function")
            wco.ongeometrychange.call(wco, ev);
          wco.dispatchEvent(ev);
          break;
        }
        case "mediachange":
          break;
        default:
          emit(appWindow, kind);
          break;
      }
    }
    for (const mql of liveQueries)
      mql.__vnReevaluate();
    return true;
  });
  globalThis.__VN_WINDOW__ = {
    appWindow,
    defineWindow,
    getCurrentWindow: () => appWindow,
    refresh,
    warn,
    controlHitboxes: (windowId) => JSON.parse(__vn_window_hitboxes(windowId)),
    cursorPos: () => JSON.parse(__vn_window_cursor()),
    state: () => S
  };
})();
