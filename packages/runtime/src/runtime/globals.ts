import { mark, vacant } from "./realm";
import { warnCompat } from "../compat";

// Missing globals are ReferenceErrors, worse than a wrong answer: reka-ui's FocusScope constructs a
// MutationObserver in a setup-time watchEffect, so every modal would die before rendering. Installed
// unconditionally when the engine lacks them.

// A previous runtime's install counts as vacant (see `REALM`): a same-realm reload replaces it.
export const defineGlobal = (name: string, value: unknown) => {
  if (vacant(globalThis, name)) {
    Object.defineProperty(globalThis, name, { value: mark(value), configurable: true, writable: true });
  }
};

// The `CSS` namespace. reka-ui's `Presence` calls `CSS.escape` inside an event handler; without it the exit
// animation never completes and the element never unmounts. `escape` is CSSOM's algorithm, not an
// approximation: it feeds a string comparison, and an under-escaped identifier silently stops matching.
defineGlobal("CSS", {
  escape(value: unknown) {
    const s = String(value);
    let out = "";
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      const ch = s[i];
      if (c === 0) {
        out += "�";
      } else if (
        (c >= 0x1 && c <= 0x1f) ||
        c === 0x7f ||
        (i === 0 && c >= 0x30 && c <= 0x39) ||
        (i === 1 && c >= 0x30 && c <= 0x39 && s.charCodeAt(0) === 0x2d)
      ) {
        out += "\\" + c.toString(16) + " ";
      } else if (i === 0 && c === 0x2d && s.length === 1) {
        out += "\\" + ch;
      } else if (
        c >= 0x80 ||
        c === 0x2d ||
        c === 0x5f ||
        (c >= 0x30 && c <= 0x39) ||
        (c >= 0x41 && c <= 0x5a) ||
        (c >= 0x61 && c <= 0x7a)
      ) {
        out += ch;
      } else {
        out += "\\" + ch;
      }
    }
    return out;
  },
  // A feature probe: answers what Skathi can express (the properties the cascade maps), since `@supports`
  // fallbacks are chosen with it.
  supports(a: unknown, b?: unknown) {
    const body = b === undefined ? String(a) : `${String(a)}:${String(b)}`;
    const colon = body.indexOf(":");
    const name = (colon < 0 ? body : body.slice(0, colon)).trim().toLowerCase();
    if (!name) return false;
    if (name.startsWith("--")) return true;
    if (!CSS_SUPPORTED.has(name)) return false;
    return colon < 0 || !CSS_UNSUPPORTED_VALUE.test(body.slice(colon + 1));
  },
  registerProperty() {
    warnCompat(
      "CSS.registerProperty",
      "CSS.registerProperty() is a no-op — Skathi's custom properties are untyped strings, " +
        "so a registered syntax/inherits/initialValue has nothing to enforce",
    );
  },
});

/// Properties `CSS.supports` answers `true` for. Conservative: a probe that
/// says yes and then does nothing is worse than one that says no.
const CSS_SUPPORTED = new Set<string>([
  "animation",
  "animation-delay",
  "animation-direction",
  "animation-duration",
  "animation-fill-mode",
  "animation-iteration-count",
  "animation-name",
  "animation-play-state",
  "animation-timing-function",
  "transition",
  "transition-delay",
  "transition-duration",
  "transition-property",
  "transition-timing-function",
  "align-items",
  "align-self",
  "background",
  "background-color",
  "border",
  "border-color",
  "border-radius",
  "border-width",
  "bottom",
  "box-shadow",
  "color",
  "column-gap",
  "display",
  "flex",
  "flex-basis",
  "flex-direction",
  "flex-grow",
  "flex-shrink",
  "flex-wrap",
  "font-size",
  "font-style",
  "font-weight",
  "gap",
  "height",
  "inset",
  "justify-content",
  "left",
  "line-height",
  "margin",
  "margin-bottom",
  "margin-left",
  "margin-right",
  "margin-top",
  "max-height",
  "max-width",
  "min-height",
  "min-width",
  "opacity",
  "overflow",
  "overflow-x",
  "overflow-y",
  "padding",
  "padding-bottom",
  "padding-left",
  "padding-right",
  "padding-top",
  "pointer-events",
  "position",
  "right",
  "row-gap",
  "text-align",
  "top",
  "transform",
  "visibility",
  "white-space",
  "width",
  "z-index",
]);

// Values `CSS.supports` rejects for a known property: `linear(…)` easing, which the parser lacks (stylesheets drop it,
// `el.animate()` refuses it), so apps probe and fall back to `cubic-bezier`.
const CSS_UNSUPPORTED_VALUE = /(?:^|[^\w-])linear\(/i;
