// Browser-standard window APIs (title, screen, geometry, matchMedia, Fullscreen, window controls
// overlay) and the `appWindow` extension (`globalThis.__SK_WINDOW__`, re-exported by runtime.ts),
// over the host's `__sk_window_*` bindings. Generated into window.prelude.js (`bun run preludes`).
//
// `document!` is always defined here, though engine.d.ts types it optional for the unit-test harness.
//
// TODO(redjs): move this into native accessors in redjs's webapi/dom.rs.
(() => {
  // The generator re-emits this at the top of the artifact.
  "use strict";

  const warn = (key: string, msg: string) => {
    if (__sk_advise(key, msg)) __sk_window_advise(msg);
  };

  // One snapshot per frame, pushed by the host; `refresh()` re-reads it, since geometry writes lag a frame.
  let raw = __sk_window_state();
  let S: SkWindowState = JSON.parse(raw);
  const changeListeners = new Set<() => void>();
  const changed = () => {
    for (const fn of changeListeners) {
      try {
        fn();
      } catch (e) {
        console.error(e);
      }
    }
  };
  const publish = (json: string) => {
    if (json === raw) return;
    raw = json;
    S = JSON.parse(json);
    changed();
  };
  const refresh = (): SkWindowState => {
    publish(__sk_window_state());
    return S;
  };

  const ro = (obj: object, name: string, get: () => unknown) =>
    Object.defineProperty(obj, name, { get, configurable: true, enumerable: true });

  const normalizeTitle = (v: unknown) => {
    const text = String(v);
    let result = "";
    let start = 0;
    for (let i = 0; i <= text.length; i++) {
      if (i < text.length) {
        const code = text.charCodeAt(i);
        // HTML ASCII whitespace is TAB, LF, FF, CR and SPACE.
        if (code !== 9 && code !== 10 && code !== 12 && code !== 13 && code !== 32) continue;
      }
      if (start < i) {
        if (result.length) result += " ";
        result += text.slice(start, i);
      }
      start = i + 1;
    }
    return result;
  };
  let titleSlot = normalizeTitle(S.title);
  Object.defineProperty(document, "title", {
    get: () => titleSlot,
    set(v: unknown) {
      const next = normalizeTitle(v);
      // The raw string goes to the OS.
      __sk_window_set_title(String(v));
      if (next === titleSlot) return;
      titleSlot = next;
      changed();
    },
    configurable: true,
    enumerable: true,
  });

  ro(globalThis, "screenX", () => S.outer.x);
  ro(globalThis, "screenY", () => S.outer.y);
  ro(globalThis, "screenLeft", () => S.outer.x);
  ro(globalThis, "screenTop", () => S.outer.y);

  // Best-effort and clamped to the display's working area; a refused call is a silent no-op.
  const BOUNDS_X = 1,
    BOUNDS_Y = 2,
    BOUNDS_W = 4,
    BOUNDS_H = 8,
    BOUNDS_CONTENT = 16;
  const finite = (v: unknown) => (typeof v === "number" && isFinite(v) ? v : undefined);
  const setBounds = (r: Partial<SkWindowRect> | null | undefined, content: boolean) => {
    const x = finite(r && r.x),
      y = finite(r && r.y);
    const w = finite(r && r.width),
      h = finite(r && r.height);
    let mask = content ? BOUNDS_CONTENT : 0;
    if (x !== undefined) mask |= BOUNDS_X;
    if (y !== undefined) mask |= BOUNDS_Y;
    if (w !== undefined) mask |= BOUNDS_W;
    if (h !== undefined) mask |= BOUNDS_H;
    __sk_window_set_bounds(x ?? 0, y ?? 0, w ?? 0, h ?? 0, mask);
  };
  // The outer size, as on the web.
  globalThis.resizeTo = (w, h) => setBounds({ width: w, height: h }, false);
  globalThis.resizeBy = (dw, dh) =>
    setBounds({ width: S.outer.width + (dw || 0), height: S.outer.height + (dh || 0) }, false);
  globalThis.moveTo = (x, y) => setBounds({ x, y }, false);
  globalThis.moveBy = (dx, dy) => setBounds({ x: S.outer.x + (dx || 0), y: S.outer.y + (dy || 0) }, false);

  // `pagehide` fires once per session, on any close or quit, as on any unload. `beforeunload` is not
  // asked again.
  let pageHidden = false;
  const hidePage = () => {
    if (pageHidden) return;
    pageHidden = true;
    globalThis.dispatchEvent(new Event("pagehide"));
  };
  const closeWindow = () => {
    hidePage();
    __sk_window_verb("close");
  };
  globalThis.close = closeWindow;
  // The host does not steal focus unless configured to, so `focus()` requests attention instead.
  globalThis.focus = () => {
    const raise = S.focusPolicy === "raise";
    if (!raise) {
      warn(
        "focus",
        "`window.focus()` requested attention instead of raising the window — this host is " +
          'configured not to steal focus. Set `window.focus: "raise"` in sk.config.json (or ' +
          "SK_WINDOW_FOCUS=raise) to allow raising.",
      );
    }
    __sk_window_activate(raise);
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
  const orientation = new EventTarget() as ScreenOrientation;
  ro(orientation, "type", () => (S.screen.width >= S.screen.height ? "landscape-primary" : "portrait-primary"));
  ro(orientation, "angle", () => 0);
  orientation.lock = () => {
    warn(
      "orientation.lock",
      "`ScreenOrientation.lock()` rejects with NotSupportedError, as it does in every desktop browser.",
    );
    const e = new Error("screen.orientation.lock() is not available on this device.");
    e.name = "NotSupportedError";
    return Promise.reject(e);
  };
  orientation.unlock = () => {};
  const screenObj = new EventTarget() as Screen;
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

  // redjs's MediaQueryList never matches; this bridges to the sk-css evaluator `@media` uses.
  const liveQueries = new Set<MediaQueryList>();
  const evaluate = (q: string) => {
    const r: SkMatchMediaResult = JSON.parse(__sk_window_match_media(q));
    if (r.error) warn(`mq:${q}`, `matchMedia(${JSON.stringify(q)}) ${r.error}`);
    return !!r.matches;
  };
  globalThis.matchMedia = (query) => {
    const q = String(query);
    const mql = new EventTarget() as MediaQueryList;
    let current = evaluate(q);
    ro(mql, "media", () => q);
    ro(mql, "matches", () => current);
    mql.onchange = null;
    mql.addListener = (fn) => mql.addEventListener("change", fn);
    mql.removeListener = (fn) => mql.removeEventListener("change", fn);
    mql.__skReevaluate = () => {
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

  let fsStack: SkFullscreenTarget[] = [];
  let fsWindowWasFullscreen = false;
  let fsInitialViewport: { width: number; height: number } | null = null;
  let fsOwnerRoot = 0;
  let fsOwnerElement: SkFullscreenTarget | null = null;
  const fsOwnerIsLive = () => fsOwnerRoot === 0 || fsOwnerElement?.__skId === fsOwnerRoot;
  const fsWindowState = () =>
    !fsOwnerIsLive()
      ? false
      : typeof __sk_fullscreen_window === "function"
        ? __sk_fullscreen_window(fsOwnerRoot)
        : refresh().fullscreen;
  const setFsWindow = (on: boolean) => {
    if (!fsOwnerIsLive()) return;
    if (typeof __sk_fullscreen_window === "function") __sk_fullscreen_window(fsOwnerRoot, on);
    else if (fsOwnerRoot === 0) __sk_window_set_fullscreen(on);
  };
  let fsFrameScheduled = false;
  let fsChanges: { target: SkFullscreenTarget | undefined; resolve: () => void }[] = [];
  let fsObserver: { observe(target: unknown, options: object): void; disconnect(): void } | null = null;
  ro(document!, "fullscreenElement", () => fsStack[fsStack.length - 1] ?? null);
  ro(document!, "fullscreenEnabled", () => true);
  const nextFsFrame = (callback: () => void) => {
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(callback);
    else setTimeout(callback, 0);
  };
  const fsBox = (target: SkFullscreenTarget | null | undefined) =>
    typeof target?.getBoundingClientRect === "function" ? target.getBoundingClientRect() : null;
  const fsViewport = () => fsBox(fsOwnerElement ?? (document!.documentElement as SkFullscreenTarget | undefined));
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
      if (
        target &&
        native.clientWidth > 0 &&
        native.clientHeight > 0 &&
        !sameFsSize(viewport, { width: native.clientWidth, height: native.clientHeight })
      )
        return false;
      // The native resize flag can precede the retained root's next paint.
      // Wait for the subsequent viewport publication as well as the layout.
      if (
        globalThis.innerWidth > 0 &&
        globalThis.innerHeight > 0 &&
        !sameFsSize(viewport, { width: globalThis.innerWidth, height: globalThis.innerHeight })
      )
        return false;
    }
    if (target) {
      const bounds = fsBox(target);
      if (bounds && viewport && !sameFsSize(bounds, viewport)) return false;
    } else if (!fsWindowWasFullscreen && fsInitialViewport && viewport && !sameFsSize(viewport, fsInitialViewport)) {
      return false;
    }
    return true;
  };
  const fireFs = (target: SkFullscreenTarget | undefined) => {
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
  const queueFsChange = (target: SkFullscreenTarget | undefined) => {
    const promise = new Promise<void>((resolve) => fsChanges.push({ target, resolve }));
    scheduleFsChanges();
    return promise;
  };
  const syncFsElement = () => {
    const target = fsStack[fsStack.length - 1];
    if (typeof __sk_fullscreen_element === "function") __sk_fullscreen_element(target?.__skId ?? 0);
    if (!target) {
      fsObserver?.disconnect();
      fsObserver = null;
    }
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
    const Observer = (
      globalThis as unknown as {
        MutationObserver?: new (callback: () => void) => {
          observe(target: unknown, options: object): void;
          disconnect(): void;
        };
      }
    ).MutationObserver;
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
  const requestFullscreen = function (this: SkFullscreenTarget, options?: FullscreenOptions) {
    const el = this;
    if (!el || el.isConnected === false || !el.__skId) {
      return Promise.reject(new TypeError("Fullscreen requires a connected element"));
    }
    if (fsStack[fsStack.length - 1] === el) return Promise.resolve();
    if (options?.keyboardLock) warn("fs.keyboardLock", "`keyboardLock` is ignored — there is no browser UI to lock.");
    if (options?.screen) warn("fs.screen", "The `screen` option needs the Window Management API.");
    const ownerElement =
      el.ownerDocument && el.ownerDocument !== document ? (el.ownerDocument.documentElement ?? null) : null;
    const ownerRoot = ownerElement?.__skId ?? 0;
    if (ownerElement && !ownerRoot) return Promise.reject(new TypeError("This element's window was closed"));
    if (fsStack.length && ownerRoot !== fsOwnerRoot) {
      return Promise.reject(new TypeError("Exit the current window fullscreen before entering another window"));
    }
    if (typeof __sk_fullscreen_element === "function" && !__sk_fullscreen_element(el.__skId)) {
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
  if (document!.documentElement)
    (document!.documentElement as SkFullscreenTarget).requestFullscreen = requestFullscreen;

  // The area available to app content; `x` is non-zero when the controls sit on the left.
  // redjs creates `navigator` lazily.
  const nav = (globalThis.navigator ||= {} as Navigator);
  const wco = new EventTarget() as WindowControlsOverlay;
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
      },
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
  // `window.open` means a second document, which is not what `<SkWindow>` (one tree, two windows) is.
  globalThis.open = () => {
    warn(
      "window.open",
      "`window.open()` is not supported: it means a SECOND DOCUMENT. To paint part of THIS tree in " +
        "a second OS window — a tool palette, a detached player — use `<SkWindow>` from " +
        '"@skathi-native/runtime".',
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
  globalThis.__skStickyActivation = markSticky;

  const appWindow = new EventTarget() as SkAppWindow;
  const verb = (name: string) => () => __sk_window_verb(name);
  appWindow.minimize = verb("minimize");
  appWindow.maximize = verb("maximize");
  appWindow.unmaximize = verb("unmaximize");
  appWindow.toggleMaximize = verb("toggleMaximize");
  appWindow.restore = verb("unmaximize");
  appWindow.close = closeWindow;
  appWindow.center = verb("center");
  appWindow.show = verb("show");
  appWindow.hide = verb("hide");
  appWindow.startDragging = verb("startDragging");
  appWindow.setFullscreen = (on) => __sk_window_set_fullscreen(!!on);
  appWindow.isMaximized = () => refresh().maximized;
  appWindow.isMinimized = () => refresh().minimized;
  appWindow.isFullscreen = () => refresh().fullscreen;
  appWindow.isFocused = () => refresh().focused;
  appWindow.isVisible = () => refresh().visible;
  appWindow.getBounds = () => ({ ...S.outer });
  appWindow.getContentBounds = () => ({ ...S.content });
  appWindow.setBounds = (r) => setBounds(r, false);
  appWindow.setContentBounds = (r) => setBounds(r, true);
  // Electron's shape: `false` retires the host-drawn min/max/close cluster, an object installs it
  // (the OS performs the actions; HTMAXBUTTON arms the Windows 11 snap flyout). Under
  // `titleBarStyle: "default"` the OS already draws real buttons, so that case is refused.
  const setOverlay = (height: unknown, mode: number, color?: unknown, symbolColor?: unknown) => {
    __sk_window_set_overlay(
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
      "`startResizeDragging()` is not implemented — the window already has live resize borders on " + "all four edges.",
    );
  for (const name of ["setProgressBar", "setOverlayIcon", "setSkipTaskbar", "setIcon"] as const) {
    appWindow[name] = () =>
      warn(
        `taskbar:${name}`,
        `\`${name}()\` is not implemented in v1 (Windows taskbar integration, no web equivalent).`,
      );
  }
  appWindow.setBadgeCount = () =>
    warn(
      "taskbar:setBadgeCount",
      "`setBadgeCount()` is not implemented. Use the web's `navigator.setAppBadge(count)` and " +
        "`navigator.clearAppBadge()`, which badge the taskbar button on Windows and the Dock icon on macOS.",
    );
  Object.defineProperty(appWindow, "title", {
    get: () => document!.title,
    set: (v: string) => (document!.title = v),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(appWindow, "backdrop", {
    get: () => S.backdrop,
    set: (v: unknown) => {
      __sk_window_set_backdrop(String(v));
      refresh();
    },
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
    set: (v: unknown) => {
      __sk_window_set_theme(String(v));
      refresh();
    },
    configurable: true,
    enumerable: true,
  });
  Object.defineProperty(appWindow, "alwaysOnTop", {
    get: () => S.alwaysOnTop,
    set: (v: unknown) => {
      __sk_window_set_always_on_top(!!v);
      refresh();
    },
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

  // `sk.config.json` `window` keys read only when the window is created.
  const CREATION_ONLY = [
    "width",
    "height",
    "minWidth",
    "minHeight",
    "x",
    "y",
    "center",
    "monitor",
    "resizable",
    "minimizable",
    "maximizable",
    "titleBarFallback",
    "focus",
  ];
  const RUNTIME_KEYS = ["title", "backdrop", "theme", "alwaysOnTop", "titleBarHeight", "titleBarOverlay"];
  // These need a different window style; changing it mid-session desyncs gpui's border_offset and
  // hit-test flags.
  const STYLE_ONLY = ["frame", "titleBarStyle"];
  const defineWindow = (opts: Record<string, unknown> | null | undefined) => {
    if (!opts || typeof opts !== "object") return;
    for (const key of Object.keys(opts)) {
      if (key === "title") {
        document!.title = opts.title as string;
        continue;
      }
      if (key === "backdrop") {
        appWindow.backdrop = opts.backdrop as string;
        continue;
      }
      if (key === "theme") {
        appWindow.theme = opts.theme as string;
        continue;
      }
      if (key === "alwaysOnTop") {
        appWindow.alwaysOnTop = opts.alwaysOnTop as boolean;
        continue;
      }
      if (key === "titleBarHeight") {
        setOverlay(opts.titleBarHeight, -1);
        continue;
      }
      if (key === "titleBarOverlay") {
        appWindow.setTitleBarOverlay(opts.titleBarOverlay as boolean);
        continue;
      }
      if (STYLE_ONLY.includes(key)) {
        warn(
          `style-only:${key}`,
          `\`${key}\` cannot change after the window is created — it needs a different window ` +
            "style, which would desync the hit test and flicker. Declare it in sk.config.json.",
        );
        continue;
      }
      if (CREATION_ONLY.includes(key)) {
        warn(
          `creation-only:${key}`,
          `\`${key}\` can only be set when the window is created. Declare it in \`sk.config.json\` under \`window\`.`,
        );
        continue;
      }
      warn(
        `defineWindow:${key}`,
        `unknown \`defineWindow\` key: ${JSON.stringify(key)}. It applies ${RUNTIME_KEYS.join(", ")}; ` +
          `${[...CREATION_ONLY, ...STYLE_ONLY].join(", ")} belong in \`sk.config.json\` under \`window\`.`,
      );
    }
  };

  // One callback, called with a comma-separated kind list and the new snapshot.
  // Its boolean return is the close veto.
  // `close-requested` listeners keyed as `addEventListener` dedupes them (listener, capture), each
  // mapped to its `once` flag.
  const closeRequestedListeners = new Map<string, boolean>();
  const listenerIds = new WeakMap<object, number>();
  let nextListenerId = 0;
  const listenerKey = (fn: unknown, opts: unknown) => {
    if (!fn || (typeof fn !== "function" && typeof fn !== "object")) return null;
    let id = listenerIds.get(fn);
    if (id === undefined) listenerIds.set(fn, (id = ++nextListenerId));
    const capture = typeof opts === "boolean" ? opts : !!(opts && (opts as { capture?: unknown }).capture);
    return `${id}:${capture}`;
  };
  const origAdd = appWindow.addEventListener.bind(appWindow);
  const origRemove = appWindow.removeEventListener.bind(appWindow);
  appWindow.addEventListener = (type, fn, opts) => {
    const key = type === "close-requested" ? listenerKey(fn, opts) : null;
    if (key && !closeRequestedListeners.has(key))
      closeRequestedListeners.set(key, !!(opts && typeof opts === "object" && opts.once));
    return origAdd(type, fn, opts);
  };
  appWindow.removeEventListener = (type, fn, opts) => {
    const key = type === "close-requested" ? listenerKey(fn, opts) : null;
    if (key) closeRequestedListeners.delete(key);
    return origRemove(type, fn, opts);
  };

  const emit = (target: SkDispatchTarget, name: string, init?: EventInit) =>
    target.dispatchEvent(new Event(name, init));

  __sk_window_observe((kinds, json) => {
    publish(json);
    // "sync" publishes the snapshot before the trusted `resize`, so getters are current in handlers.
    if (kinds === "sync") return true;
    // A close or quit that asks nothing: the tray's quit item, the embedder, the session ending.
    if (kinds === "pagehide") {
      hidePage();
      return true;
    }
    if (kinds === "close-requested") {
      // The user asked to close (title bar, Alt+F4, taskbar). `close()`, the tray's quit item and
      // the session ending close without asking. Not gated on sticky activation: an untouched
      // app may still keep its window, e.g. to hide it to a tray.
      const request = new Event("close-requested", { cancelable: true });
      appWindow.dispatchEvent(request);
      for (const [key, once] of closeRequestedListeners) if (once) closeRequestedListeners.delete(key);
      if (request.defaultPrevented) return false;
      const ev = new Event("beforeunload", { cancelable: true });
      globalThis.dispatchEvent(ev);
      if (!sticky || !ev.defaultPrevented) {
        hidePage();
        return true;
      }
      if (closeRequestedListeners.size === 0) {
        warn(
          "beforeunload-trap",
          "`beforeunload` cancelled the close, but Skathi shows no confirmation dialog, so the " +
            "close button does nothing. Cancel `close-requested` on `appWindow` instead, ask the " +
            "user, and call `appWindow.close()` when they confirm.",
        );
      }
      return false;
    }
    for (const kind of kinds.split(",")) {
      switch (kind) {
        case "boot":
          break;
        // The window's own `resize` was dispatched by the host before this call.
        case "resize":
          emit(appWindow, kind);
          break;
        case "focus":
        case "blur":
          emit(globalThis, kind);
          emit(appWindow, kind);
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
    for (const mql of liveQueries) mql.__skReevaluate();
    return true;
  });

  globalThis.__SK_WINDOW__ = {
    appWindow,
    escapeFullscreen: exitAllFullscreen,
    defineWindow,
    getCurrentWindow: () => appWindow,
    onChange: (fn: () => void) => {
      changeListeners.add(fn);
      return () => void changeListeners.delete(fn);
    },
    refresh,
    warn,
    // Debug hook: WM_NCHITTEST cannot be synthesised, so this reports the last painted frame's hitboxes.
    controlHitboxes: (windowId?: number): SkWindowHitbox[] => JSON.parse(__sk_window_hitboxes(windowId)),
    cursorPos: (): SkWindowCursorPos => JSON.parse(__sk_window_cursor()),
    state: () => S,
  };
})();
