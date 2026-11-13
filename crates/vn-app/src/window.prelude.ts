// Browser-standard window APIs (title, screen, geometry, matchMedia, Fullscreen, window controls
// overlay) and the `appWindow` extension (`globalThis.__VN_WINDOW__`, re-exported by runtime.ts),
// over the host's `__vn_window_*` bindings. Generated into window.prelude.js (`bun run preludes`).
//
// `document!` is always defined here, though engine.d.ts types it optional for the unit-test harness.
//
// TODO(redjs): move this into native accessors in redjs's webapi/dom.rs.
(() => {
  // The generator re-emits this at the top of the artifact. Sloppy mode would turn the guarded
  // `globalThis.innerWidth` seeding below from a throw into a silent no-op.
  "use strict";

  const warned = new Set<string>();
  const warn = (key: string, msg: string) => {
    if (warned.has(key)) return;
    warned.add(key);
    console.warn(`[vue-native compat] ${msg}`);
    __vn_window_advise(msg);
  };

  // One snapshot per frame, pushed by the host; `refresh()` re-reads it, since geometry writes lag a frame.
  let S: VnWindowState = JSON.parse(__vn_window_state());
  const refresh = (): VnWindowState => (S = JSON.parse(__vn_window_state()));

  const ro = (obj: object, name: string, get: () => unknown) =>
    Object.defineProperty(obj, name, { get, configurable: true, enumerable: true });

  const normalizeTitle = (v: unknown) => String(v).replace(/[\t\n\f\r ]+/g, " ").replace(/^ | $/g, "");
  let titleSlot = normalizeTitle(S.title);
  Object.defineProperty(document, "title", {
    get: () => titleSlot,
    set(v: unknown) {
      titleSlot = normalizeTitle(v);
      // The raw string goes to the OS.
      __vn_window_set_title(String(v));
    },
    configurable: true,
    enumerable: true,
  });

  // `outerHeight - innerHeight` is not the titlebar height: use `getTitlebarAreaRect().height`.
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

  // Best-effort and clamped to the display's working area; a refused call is a silent no-op.
  const BOUNDS_X = 1, BOUNDS_Y = 2, BOUNDS_W = 4, BOUNDS_H = 8, BOUNDS_CONTENT = 16;
  const finite = (v: unknown) => (typeof v === "number" && isFinite(v) ? v : undefined);
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
  globalThis.resizeTo = (w, h) => setBounds({ width: w, height: h }, true);
  globalThis.resizeBy = (dw, dh) =>
    setBounds({ width: S.innerWidth + (dw || 0), height: S.innerHeight + (dh || 0) }, true);
  globalThis.moveTo = (x, y) => setBounds({ x, y }, false);
  globalThis.moveBy = (dx, dy) =>
    setBounds({ x: S.outer.x + (dx || 0), y: S.outer.y + (dy || 0) }, false);

  // `pagehide` fires once per close (a WM_CLOSE the app did not veto, or `close()`), as on any
  // unload. `beforeunload` is not asked again.
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
  // The host does not steal focus unless configured to, so `focus()` requests attention instead.
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
  // A no-op in browsers too; warned so it is not misread as a bug.
  globalThis.blur = () =>
    warn(
      "blur",
      "`window.blur()` did nothing — and does nothing in any browser since it was removed as a " +
        "popunder vector. There is no web API to send a window behind others.",
    );
  document!.hasFocus = () => S.focused;

  // Exact values, not fuzzed. redjs has no Screen, MediaQueryList or WindowControlsOverlay class, so
  // each is an EventTarget grown into the interface; the `as` casts name the end result.
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
  ro(screenObj, "colorDepth", () => 24);
  ro(screenObj, "pixelDepth", () => 24);
  ro(screenObj, "isExtended", () => S.screen.isExtended);
  ro(screenObj, "orientation", () => orientation);
  globalThis.screen = screenObj;
  // Needs multi-window placement: rejects with an advisory.
  globalThis.getScreenDetails = () => {
    warn(
      "getScreenDetails",
      "`getScreenDetails()` is not implemented (v1 is single-window). `screen.isExtended` reports " +
        "whether more than one display exists.",
    );
    const e = new Error("getScreenDetails() is not supported.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };

  // redjs's MediaQueryList never matches; this bridges to the vn-css evaluator `@media` uses.
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
    mql.addListener = (fn) => mql.addEventListener("change", fn);
    mql.removeListener = (fn) => mql.removeEventListener("change", fn);
    mql.__vnReevaluate = () => {
      const next = evaluate(q);
      if (next === current) return;
      current = next;
      const ev = new Event("change");
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

  let fsStack: VnFullscreenTarget[] = [];
  let fsWindowWasFullscreen = false;
  let fsInitialViewport: { width: number; height: number } | null = null;
  let fsOwnerRoot = 0;
  let fsOwnerElement: VnFullscreenTarget | null = null;
  const fsOwnerIsLive = () => fsOwnerRoot === 0 || fsOwnerElement?.__vnId === fsOwnerRoot;
  const fsWindowState = () => !fsOwnerIsLive() ? false : typeof __vn_fullscreen_window === "function"
    ? __vn_fullscreen_window(fsOwnerRoot) : refresh().fullscreen;
  const setFsWindow = (on: boolean) => {
    if (!fsOwnerIsLive()) return;
    if (typeof __vn_fullscreen_window === "function") __vn_fullscreen_window(fsOwnerRoot, on);
    else if (fsOwnerRoot === 0) __vn_window_set_fullscreen(on);
  };
  let fsFrameScheduled = false;
  let fsChanges: { target: VnFullscreenTarget | undefined; resolve: () => void }[] = [];
  let fsObserver: { observe(target: unknown, options: object): void; disconnect(): void } | null = null;
  ro(document!, "fullscreenElement", () => fsStack[fsStack.length - 1] ?? null);
  ro(document!, "fullscreenEnabled", () => true);
  const nextFsFrame = (callback: () => void) => {
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(callback);
    else setTimeout(callback, 0);
  };
  const fsBox = (target: VnFullscreenTarget | null | undefined) =>
    typeof target?.getBoundingClientRect === "function" ? target.getBoundingClientRect() : null;
  const fsViewport = () => fsBox(fsOwnerElement ?? (document!.documentElement as VnFullscreenTarget | undefined));
  const sameFsSize = (a: { width: number; height: number }, b: { width: number; height: number }) =>
    Math.abs(a.width - b.width) < 1 && Math.abs(a.height - b.height) < 1;
  const fsLayoutReady = () => {
    if (!fsOwnerIsLive()) return true;
    const target = fsStack[fsStack.length - 1];
    if (fsWindowState() !== (!!target || fsWindowWasFullscreen)) return false;
    const viewport = fsViewport();
    if (viewport && (viewport.width <= 0 || viewport.height <= 0)) return false;
    if (viewport && fsOwnerRoot === 0) {
      const native = refresh();
      if (target && native.clientWidth > 0 && native.clientHeight > 0 &&
          !sameFsSize(viewport, { width: native.clientWidth, height: native.clientHeight })) return false;
      // The native resize flag can precede the retained root's next paint.
      // Wait for the subsequent viewport publication as well as the layout.
      if (globalThis.innerWidth > 0 && globalThis.innerHeight > 0 &&
          !sameFsSize(viewport, { width: globalThis.innerWidth, height: globalThis.innerHeight })) return false;
    }
    if (target) {
      const bounds = fsBox(target);
      if (bounds && viewport && !sameFsSize(bounds, viewport)) return false;
    } else if (!fsWindowWasFullscreen && fsInitialViewport && viewport && !sameFsSize(viewport, fsInitialViewport)) {
      return false;
    }
    return true;
  };
  const fireFs = (target: VnFullscreenTarget | undefined) => {
    if (target?.isConnected !== false && typeof target?.dispatchEvent === "function") {
      target.dispatchEvent(new Event("fullscreenchange", { bubbles: true, composed: true }));
    } else {
      document!.dispatchEvent(new Event("fullscreenchange", { bubbles: true, composed: true }));
    }
  };
  const flushFsChanges = () => {
    if (!fsChanges.length || !fsLayoutReady()) return;
    const changes = fsChanges;
    fsChanges = [];
    for (const change of changes) {
      fireFs(change.target);
      // Vue reacts to the event on a microtask. Let that patch and the owning
      // window's paint complete before the caller reads the restored layout.
      nextFsFrame(() => nextFsFrame(change.resolve));
    }
  };
  const scheduleFsChanges = () => {
    if (fsFrameScheduled) return;
    fsFrameScheduled = true;
    nextFsFrame(() => {
      fsFrameScheduled = false;
      flushFsChanges();
      if (fsChanges.length) scheduleFsChanges();
    });
  };
  const queueFsChange = (target: VnFullscreenTarget | undefined) => {
    const promise = new Promise<void>((resolve) => fsChanges.push({ target, resolve }));
    scheduleFsChanges();
    return promise;
  };
  const syncFsElement = () => {
    const target = fsStack[fsStack.length - 1];
    if (typeof __vn_fullscreen_element === "function") __vn_fullscreen_element(target?.__vnId ?? 0);
    if (!target) { fsObserver?.disconnect(); fsObserver = null; }
  };
  const leaveFsWindow = () => {
    if (!fsStack.length && !fsWindowWasFullscreen) setFsWindow(false);
  };
  const exitAllFullscreen = () => {
    const previous = fsStack[fsStack.length - 1];
    if (!previous) return false;
    fsStack = [];
    syncFsElement();
    queueFsChange(previous);
    leaveFsWindow();
    return true;
  };
  const observeFsRemoval = () => {
    if (fsObserver) return;
    const Observer = (globalThis as unknown as { MutationObserver?: new (callback: () => void) =>
      { observe(target: unknown, options: object): void; disconnect(): void } }).MutationObserver;
    if (!Observer) return;
    fsObserver = new Observer(() => {
      const previous = fsStack[fsStack.length - 1];
      fsStack = fsStack.filter((target) => target.isConnected !== false);
      if (previous !== fsStack[fsStack.length - 1]) {
        syncFsElement();
        queueFsChange(previous);
        leaveFsWindow();
      }
    });
    fsObserver.observe(document!, { childList: true, subtree: true });
  };
  const onFullscreenWindowChanged = (fullscreen: boolean) => {
    if (fsOwnerRoot !== 0) return;
    if (!fullscreen && fsStack.length) {
      const previous = fsStack[fsStack.length - 1];
      fsStack = [];
      syncFsElement();
      if (!fsChanges.length) queueFsChange(previous);
    }
    if (fsChanges.length) scheduleFsChanges();
  };
  const requestFullscreen = function (this: VnFullscreenTarget, options?: VnFullscreenOptions) {
    const el = this;
    if (!el || el.isConnected === false || !el.__vnId) {
      return Promise.reject(new TypeError("Fullscreen requires a connected element"));
    }
    if (fsStack[fsStack.length - 1] === el) return Promise.resolve();
    if (options?.keyboardLock) warn("fs.keyboardLock", "`keyboardLock` is ignored — there is no browser UI to lock.");
    if (options?.screen) warn("fs.screen", "The `screen` option needs the Window Management API.");
    const ownerElement = el.ownerDocument && el.ownerDocument !== document
      ? el.ownerDocument.documentElement ?? null : null;
    const ownerRoot = ownerElement?.__vnId ?? 0;
    if (ownerElement && !ownerRoot) return Promise.reject(new TypeError("This element's window was closed"));
    if (fsStack.length && ownerRoot !== fsOwnerRoot) {
      return Promise.reject(new TypeError("Exit the current window fullscreen before entering another window"));
    }
    if (typeof __vn_fullscreen_element === "function" && !__vn_fullscreen_element(el.__vnId)) {
      return Promise.reject(new TypeError("This element cannot enter fullscreen in this window"));
    }
    if (!fsStack.length) {
      fsOwnerRoot = ownerRoot;
      fsOwnerElement = ownerElement;
      fsWindowWasFullscreen = fsWindowState();
      fsInitialViewport = fsViewport();
    }
    fsStack.push(el);
    observeFsRemoval();
    const promise = queueFsChange(el);
    setFsWindow(true);
    return promise;
  };
  document!.exitFullscreen = () => {
    const previous = fsStack.pop();
    if (!previous) return Promise.resolve();
    while (fsStack.length && fsStack[fsStack.length - 1]?.isConnected === false) fsStack.pop();
    syncFsElement();
    const promise = queueFsChange(previous);
    leaveFsWindow();
    return promise;
  };
  if (typeof Element === "function" && Element.prototype) Element.prototype.requestFullscreen = requestFullscreen;
  if (document!.documentElement) (document!.documentElement as VnFullscreenTarget).requestFullscreen = requestFullscreen;

  // The area available to app content; `x` is non-zero when the controls sit on the left.
  // redjs creates `navigator` lazily.
  const nav = (globalThis.navigator ||= {});
  const wco = new EventTarget() as VnWindowControlsOverlay;
  ro(wco, "visible", () => S.titleBarOverlayVisible);
  wco.getTitlebarAreaRect = () => {
    const r = S.titleBarArea;
    return {
      x: r.x, y: r.y, width: r.width, height: r.height,
      top: r.y, left: r.x, right: r.x + r.width, bottom: r.y + r.height,
      toJSON() { return { ...r }; },
    };
  };
  wco.ongeometrychange = null;
  Object.defineProperty(nav, "windowControlsOverlay", { value: wco, configurable: true, enumerable: true });

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
  // `window.open` means a second document, which is not what `<VnWindow>` (one tree, two windows) is.
  globalThis.open = () => {
    warn(
      "window.open",
      "`window.open()` is not supported: it means a SECOND DOCUMENT. To paint part of THIS tree in " +
        "a second OS window — a tool palette, a detached player — use `<VnWindow>` from " +
        '"vue-native".',
    );
    return null;
  };

  // Sticky activation gates the `beforeunload` veto, as in Chrome: an untouched app cannot refuse to close.
  let sticky = false;
  const markSticky = () => (sticky = true);
  if (addEL) {
    addEL("keydown", markSticky, true);
    addEL("mousedown", markSticky, true);
  }
  globalThis.__vnStickyActivation = markSticky;

  const appWindow = new EventTarget() as VnAppWindow;
  const verb = (name: string) => () => __vn_window_verb(name);
  appWindow.minimize = verb("minimize");
  appWindow.maximize = verb("maximize");
  appWindow.unmaximize = verb("unmaximize");
  appWindow.toggleMaximize = verb("toggleMaximize");
  appWindow.restore = verb("unmaximize");
  appWindow.close = closeWindow;
  appWindow.center = verb("center");
  appWindow.show = verb("show");
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
  // Electron's shape: `false` retires the host-drawn min/max/close cluster, an object installs it
  // (the OS performs the actions; HTMAXBUTTON arms the Windows 11 snap flyout). Under
  // `titleBarStyle: "default"` the OS already draws real buttons, so that case is refused.
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
    // An object means "install the overlay"; `titleBarHeight` alone never does.
    setOverlay(o.height, 1, o.color, o.symbolColor);
  };
  appWindow.startResizeDragging = () =>
    warn(
      "startResizeDragging",
      "`startResizeDragging()` is not implemented — the window already has live resize borders on " +
        "all four edges.",
    );
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
  // What the window draws of `backdrop`: `none` while a macOS fullscreen window has no desktop
  // picture. `backdropchange` fires when either moves.
  ro(appWindow, "effectiveBackdrop", () => S.effectiveBackdrop);
  // Electron's `nativeTheme.themeSource`: the windows' dark mode (Mica's tint)
  // and `prefers-color-scheme`. Read back as `backdrop` is.
  Object.defineProperty(appWindow, "theme", {
    get: () => S.theme,
    set: (v: unknown) => { __vn_window_set_theme(String(v)); refresh(); },
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
  ro(appWindow, "chrome", () => ({
    titleBarStyle: S.titleBarStyle,
    displayMode: S.displayMode,
    titleBarHeight: S.titleBarArea.height,
    fallbackTitleBar: S.fallbackTitleBar,
    overlayVisible: S.titleBarOverlayVisible,
    hostControls: S.hostControls,
    // The client area and how far host-drawn chrome insets the viewport inside it:
    // `inner* === client* - inset`, and the inset is zero in custom-chrome mode.
    client: { width: S.clientWidth, height: S.clientHeight },
    contentInset: { ...S.contentInset },
  }));

  // gpui fixes these at window creation.
  const CREATION_ONLY = [
    "width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight",
    "resizable", "minimizable", "maximizable", "closable", "movable",
    "titleBarFallback", "focus",
  ];
  // These need a different window style; changing it mid-session desyncs gpui's border_offset and
  // hit-test flags.
  const STYLE_ONLY = ["frame", "titleBarStyle"];
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
            "`apps/<app>/vn.config.json` under `window`.",
        );
        continue;
      }
      warn(`defineWindow:${key}`, `unknown \`defineWindow\` key: ${JSON.stringify(key)}`);
    }
  };

  // One callback, called with a comma-separated kind list and the new snapshot.
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
    // "sync" publishes the snapshot before the trusted `resize`, so getters are current in handlers.
    if (kinds === "sync") return true;
    if (kinds === "close-requested") {
      // A cancelled `beforeunload` vetoes the close and raises `close-requested`; no dialog is shown.
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
          break;
        case "focus":
        case "blur":
          emit(globalThis, kind);
          break;
        case "fullscreenchange":
          onFullscreenWindowChanged(S.fullscreen);
          // Window chrome follows native fullscreen even when no DOM element
          // owns fullscreen (for example AppKit's green traffic-light button).
          emit(appWindow, kind);
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
          emit(appWindow, kind);
          break;
      }
    }
    for (const mql of liveQueries) mql.__vnReevaluate();
    return true;
  });

  globalThis.__VN_WINDOW__ = {
    appWindow,
    escapeFullscreen: exitAllFullscreen,
    defineWindow,
    getCurrentWindow: () => appWindow,
    refresh,
    warn,
    // Debug hook: WM_NCHITTEST cannot be synthesised, so this reports the last painted frame's hitboxes.
    controlHitboxes: (windowId?: number): VnWindowHitbox[] =>
      JSON.parse(__vn_window_hitboxes(windowId)),
    cursorPos: (): VnWindowCursorPos => JSON.parse(__vn_window_cursor()),
    state: () => S,
  };
})();
