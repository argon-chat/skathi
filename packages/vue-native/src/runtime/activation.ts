import { mark, vacant } from "./realm";
import { shellHost } from "../appapi";
import type { AppDesktopError } from "../appapi";
import { warnCompat } from "../compat";
import { CONTENT_SCHEMES, SCHEME_RE, adviseScheme } from "./links";

// Transient activation: a keydown or mousedown the host delivered in the last five seconds
// (Chromium's lifespan). `el.click()` and `dispatchEvent` never activate; neither do Escape or a
// lone modifier.
const ACTIVATION_MS = 5000;
let activationAt = -Infinity;
let hasBeenActive = false;
const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "Fn", "OS"]);
const nowMs = () => Date.now();

export function noteActivation(name: string, key: string | undefined): void {
  if (name === "keydown" && (key === "Escape" || (key !== undefined && MODIFIER_KEYS.has(key)))) return;
  activationAt = nowMs();
  hasBeenActive = true;
}

export const activation = {
  isActive: () => nowMs() - activationAt <= ACTIVATION_MS,
  consume: () => void (activationAt = -Infinity),
};

if (typeof navigator !== "undefined" && navigator && vacant(navigator, "userActivation")) {
  try {
    Object.defineProperty(navigator, "userActivation", {
      value: mark(
        Object.freeze(
          Object.create(Object.prototype, {
            isActive: { get: activation.isActive, enumerable: true },
            hasBeenActive: { get: () => hasBeenActive, enumerable: true },
          }),
        ),
      ),
      configurable: true,
    });
  } catch {
    /* a frozen navigator keeps its own shape */
  }
}

Object.defineProperty(globalThis, "__VN_ACTIVATION__", { value: Object.freeze(activation), configurable: true });

export const fsHost = typeof __VN_FS__ === "object" ? __VN_FS__ : null;

/**
 * Hands a content URL to the system. The caller's allowlist is applied again by the host, so
 * neither side alone is the security line. Never throws; a failure is a console warning.
 */
export function openContentUrl(url: string, via: string): void {
  if (!shellHost) {
    warnCompat(
      "shell:absent",
      `${via}: the desktop host is missing (vn_desktop::install did not run) — the link was not ` + `opened.`,
    );
    return;
  }
  let href = url;
  try {
    if (typeof URL === "function") href = new URL(url).href;
  } catch {
    /* not a parseable URL: the host validates and refuses */
  }
  shellHost.openContent(href).catch((e: AppDesktopError) => {
    if (e && e.code === "ESCHEME") return void adviseScheme(SCHEME_RE.exec(href)?.[1] ?? "?", via);
    console.warn(`[vue-native] ${via}: could not open ${href} — ${e && e.message}`);
  });
}

// `window.open(url)` for an external URL opens the system browser and returns `null` (noopener
// semantics). Anything else falls through to the prelude's `open`, which evaluates first.
{
  type OpenFn = (url?: unknown, target?: unknown, features?: unknown) => unknown;
  const g = globalThis as unknown as { open?: OpenFn };
  const prior = typeof g.open === "function" ? g.open : null;
  g.open = function open(url?: unknown, target?: unknown, features?: unknown) {
    const s = url == null ? "" : String(url).trim();
    const m = SCHEME_RE.exec(s);
    // A one-letter "scheme" is a drive letter (`C:\x`), not a URL.
    if (m && m[1]!.length > 1) {
      const scheme = m[1]!.toLowerCase();
      if (scheme !== "about" && scheme !== "javascript") {
        if (CONTENT_SCHEMES.has(scheme)) openContentUrl(s, "window.open()");
        else adviseScheme(scheme, "window.open()");
        return null;
      }
    }
    return prior ? prior(url, target, features) : null;
  };
}
