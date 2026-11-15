import { mark, vacant } from "./realm";

// Missing globals are ReferenceErrors, worse than a wrong answer: reka-ui's FocusScope constructs a
// MutationObserver in a setup-time watchEffect, so every modal would die before rendering. Installed
// unconditionally when the engine lacks them.

// A previous runtime's install counts as vacant (see `REALM`): a same-realm reload replaces it.
export const defineGlobal = (name: string, value: unknown) => {
  if (vacant(globalThis, name)) {
    Object.defineProperty(globalThis, name, { value: mark(value), configurable: true, writable: true });
  }
};
