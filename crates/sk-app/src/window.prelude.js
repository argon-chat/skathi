// GENERATED from crates/sk-app/src/window.prelude.ts — DO NOT EDIT.
//
//   source:   crates/sk-app/src/window.prelude.ts
//   emitter:  Bun.Transpiler (the same one build.ts uses for dist/vendor.js)
//   regen:    bun scripts/gen-preludes.ts
//   check:    bun scripts/gen-preludes.ts --check
//
// evaluated as `sk://window.js`, after the prelude and before the app bundle.
//
// Every comment in the source is gone: bun re-prints from the AST. Read the
// .ts — it is the file with the reasoning in it, and the only one an edit
// survives.
"use strict";
(() => {
  const host = globalThis.__SK_WINDOW__;
  const appWindow = host.appWindow;
  const warn = (key, msg) => {
    if (__sk_advise(key, msg))
      __sk_window_advise(msg);
  };
  const ro = (obj, name, get) => Object.defineProperty(obj, name, { get, configurable: true, enumerable: true });
  let fsStack = [];
  let fsWindowWasFullscreen = false;
  let fsInitialViewport = null;
  let fsOwnerRoot = 0;
  let fsOwnerElement = null;
  const fsOwnerIsLive = () => fsOwnerRoot === 0 || fsOwnerElement?.__skId === fsOwnerRoot;
  const fsWindowState = () => fsOwnerIsLive() && __sk_fullscreen_window(fsOwnerRoot);
  const setFsWindow = (on) => {
    if (fsOwnerIsLive())
      __sk_fullscreen_window(fsOwnerRoot, on);
  };
  let fsFrameScheduled = false;
  let fsChanges = [];
  let fsObserver = null;
  ro(document, "fullscreenElement", () => fsStack[fsStack.length - 1] ?? null);
  ro(document, "fullscreenEnabled", () => true);
  const fsBox = (target) => typeof target?.getBoundingClientRect === "function" ? target.getBoundingClientRect() : null;
  const fsViewport = () => fsBox(fsOwnerElement ?? document.documentElement);
  const sameFsSize = (a, b) => Math.abs(a.width - b.width) < 1 && Math.abs(a.height - b.height) < 1;
  const fsLayoutReady = () => {
    if (!fsOwnerIsLive())
      return true;
    const target = fsStack[fsStack.length - 1];
    if (fsWindowState() !== (!!target || fsWindowWasFullscreen))
      return false;
    const viewport = fsViewport();
    if (viewport && (viewport.width <= 0 || viewport.height <= 0))
      return false;
    if (viewport && fsOwnerRoot === 0) {
      host.refresh();
      const client = appWindow.chrome.client;
      if (target && client.width > 0 && client.height > 0 && !sameFsSize(viewport, client))
        return false;
      if (globalThis.innerWidth > 0 && globalThis.innerHeight > 0 && !sameFsSize(viewport, { width: globalThis.innerWidth, height: globalThis.innerHeight }))
        return false;
    }
    if (target) {
      const bounds = fsBox(target);
      if (bounds && viewport && !sameFsSize(bounds, viewport))
        return false;
    } else if (!fsWindowWasFullscreen && fsInitialViewport && viewport && !sameFsSize(viewport, fsInitialViewport)) {
      return false;
    }
    return true;
  };
  const fireFs = (target) => {
    if (target?.isConnected !== false && typeof target?.dispatchEvent === "function") {
      target.dispatchEvent(new Event("fullscreenchange", { bubbles: true, composed: true }));
    } else {
      document.dispatchEvent(new Event("fullscreenchange", { bubbles: true, composed: true }));
    }
  };
  const flushFsChanges = () => {
    if (!fsChanges.length || !fsLayoutReady())
      return;
    const changes = fsChanges;
    fsChanges = [];
    for (const change of changes) {
      fireFs(change.target);
      requestAnimationFrame(() => requestAnimationFrame(change.resolve));
    }
  };
  const scheduleFsChanges = () => {
    if (fsFrameScheduled)
      return;
    fsFrameScheduled = true;
    requestAnimationFrame(() => {
      fsFrameScheduled = false;
      flushFsChanges();
      if (fsChanges.length)
        scheduleFsChanges();
    });
  };
  const queueFsChange = (target) => {
    const promise = new Promise((resolve) => fsChanges.push({ target, resolve }));
    scheduleFsChanges();
    return promise;
  };
  const syncFsElement = () => {
    const target = fsStack[fsStack.length - 1];
    __sk_fullscreen_element(target?.__skId ?? 0);
    if (!target) {
      fsObserver?.disconnect();
      fsObserver = null;
    }
  };
  const leaveFsWindow = () => {
    if (!fsStack.length && !fsWindowWasFullscreen)
      setFsWindow(false);
  };
  const exitAllFullscreen = () => {
    const previous = fsStack[fsStack.length - 1];
    if (!previous)
      return false;
    fsStack = [];
    syncFsElement();
    queueFsChange(previous);
    leaveFsWindow();
    return true;
  };
  const observeFsRemoval = () => {
    if (fsObserver)
      return;
    fsObserver = new MutationObserver(() => {
      const previous = fsStack[fsStack.length - 1];
      fsStack = fsStack.filter((target) => target.isConnected !== false);
      if (previous !== fsStack[fsStack.length - 1]) {
        syncFsElement();
        queueFsChange(previous);
        leaveFsWindow();
      }
    });
    fsObserver.observe(document, { childList: true, subtree: true });
  };
  const onFullscreenWindowChanged = (fullscreen) => {
    if (fsOwnerRoot !== 0)
      return;
    if (!fullscreen && fsStack.length) {
      const previous = fsStack[fsStack.length - 1];
      fsStack = [];
      syncFsElement();
      if (!fsChanges.length)
        queueFsChange(previous);
    }
    if (fsChanges.length)
      scheduleFsChanges();
  };
  const requestFullscreen = function(options) {
    const el = this;
    if (!el || el.isConnected === false || !el.__skId) {
      return Promise.reject(new TypeError("Fullscreen requires a connected element"));
    }
    if (fsStack[fsStack.length - 1] === el)
      return Promise.resolve();
    if (options?.keyboardLock)
      warn("fs.keyboardLock", "`keyboardLock` is ignored — there is no browser UI to lock.");
    if (options?.screen)
      warn("fs.screen", "The `screen` option needs the Window Management API.");
    const ownerElement = el.ownerDocument && el.ownerDocument !== document ? el.ownerDocument.documentElement ?? null : null;
    const ownerRoot = ownerElement?.__skId ?? 0;
    if (ownerElement && !ownerRoot)
      return Promise.reject(new TypeError("This element's window was closed"));
    if (fsStack.length && ownerRoot !== fsOwnerRoot) {
      return Promise.reject(new TypeError("Exit the current window fullscreen before entering another window"));
    }
    if (!__sk_fullscreen_element(el.__skId)) {
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
  document.exitFullscreen = () => {
    const previous = fsStack.pop();
    if (!previous)
      return Promise.resolve();
    while (fsStack.length && fsStack[fsStack.length - 1]?.isConnected === false)
      fsStack.pop();
    syncFsElement();
    const promise = queueFsChange(previous);
    leaveFsWindow();
    return promise;
  };
  Element.prototype.requestFullscreen = requestFullscreen;
  if (document.documentElement)
    document.documentElement.requestFullscreen = requestFullscreen;
  appWindow.addEventListener("fullscreenchange", () => onFullscreenWindowChanged(appWindow.isFullscreen()));
  host.escapeFullscreen = exitAllFullscreen;
})();
