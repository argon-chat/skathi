// Badging (`navigator.setAppBadge` / `clearAppBadge`) on the app's taskbar or dock icon.

export interface BadgeHost {
  /** `-1` a flag (a dot), `0` none, otherwise the count. */
  set(contents: number): void;
}

const FLAG = -1;

function contentsOf(v: unknown): number {
  if (v === undefined) return FLAG;
  const n = Number(v);
  if (!Number.isFinite(n)) {
    throw new TypeError("Failed to execute 'setAppBadge' on 'Navigator': Value is not of type 'unsigned long long'.");
  }
  const t = Math.trunc(n);
  if (t < 0 || t > Number.MAX_SAFE_INTEGER) {
    throw new TypeError(
      "Failed to execute 'setAppBadge' on 'Navigator': Value is outside the 'unsigned long long' value range.",
    );
  }
  return t;
}

export function installBadge(nav: object, host: BadgeHost): void {
  Object.defineProperties(nav, {
    setAppBadge: {
      value: function setAppBadge(contents?: unknown): Promise<void> {
        try {
          host.set(contentsOf(contents));
          return Promise.resolve();
        } catch (e) {
          return Promise.reject(e);
        }
      },
      configurable: true,
      writable: true,
    },
    clearAppBadge: {
      value: function clearAppBadge(): Promise<void> {
        host.set(0);
        return Promise.resolve();
      },
      configurable: true,
      writable: true,
    },
  });
}
