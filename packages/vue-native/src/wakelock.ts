// Screen Wake Lock. The host holds one system request while any sentinel is active; every sentinel is
// released when the page is hidden (a minimized window, or one closing), as in a browser.

export interface WakeLockHost {
  /** Takes or drops the system request; false when the system refused. */
  hold(on: boolean): boolean;
}

export interface WakeLockDeps {
  host: WakeLockHost;
  engine: {
    EventTarget: new () => VnEventTarget;
    Event: new (type: string) => VnEvent;
    DOMException: new (message?: string, name?: string) => Error;
  };
  isHidden(): boolean;
  /** Calls `fn` whenever the page becomes hidden. */
  onHidden(fn: () => void): void;
}

export function installWakeLock(nav: object, deps: WakeLockDeps): void {
  const { host, isHidden } = deps;
  const { EventTarget, Event, DOMException } = deps.engine;
  const INTERNAL = Symbol("vn.wakeLock");
  const active = new Set<WakeLockSentinel>();

  const drop = (s: WakeLockSentinel): void => {
    if (!active.delete(s)) return;
    if (active.size === 0) host.hold(false);
    s.dispatchEvent(new Event("release"));
  };

  class WakeLockSentinel extends EventTarget {
    #released = false;
    onrelease: ((this: WakeLockSentinel, ev: VnEvent) => unknown) | null = null;
    constructor(key?: symbol) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
      super();
    }
    get released(): boolean {
      return this.#released;
    }
    get type(): "screen" {
      return "screen";
    }
    release(): Promise<void> {
      if (!this.#released) {
        this.#released = true;
        drop(this);
      }
      return Promise.resolve();
    }
  }

  class WakeLock {
    constructor(key?: symbol) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
    }
    request(type: unknown = "screen"): Promise<WakeLockSentinel> {
      if (type !== "screen") {
        return Promise.reject(
          new TypeError(
            `Failed to execute 'request' on 'WakeLock': The provided value '${String(type)}' is not a valid enum value of type WakeLockType.`,
          ),
        );
      }
      if (isHidden()) {
        return Promise.reject(new DOMException("The requesting page is not visible", "NotAllowedError"));
      }
      if (active.size === 0 && !host.hold(true)) {
        return Promise.reject(new DOMException("The system refused to keep the screen on", "NotAllowedError"));
      }
      const s = new WakeLockSentinel(INTERNAL);
      active.add(s);
      return Promise.resolve(s);
    }
  }

  deps.onHidden(() => {
    for (const s of [...active]) void s.release();
  });

  const g = globalThis as Record<string, unknown>;
  g.WakeLock = WakeLock;
  g.WakeLockSentinel = WakeLockSentinel;
  Object.defineProperty(nav, "wakeLock", {
    value: new WakeLock(INTERNAL),
    configurable: true,
    enumerable: true,
  });
}
