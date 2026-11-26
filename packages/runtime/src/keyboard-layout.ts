import type { HotkeyButton } from "./hotkeys";

const host = globalThis as typeof globalThis & {
  __sk_keyScans?: Readonly<Record<string, number>>;
  __sk_keyLegends?: Readonly<Record<string, string>>;
  __sk_keyCodesByVk?: Readonly<Record<number, string>>;
  __sk_keyLayoutChanged?: () => void;
};
const observers = new Set<() => void>();
const changed = () => {
  for (const observer of observers) observer();
};

/** @internal Subscribe a framework render to a native input-source change. */
export function onKeyLayoutChange(observer: () => void): () => void {
  observers.add(observer);
  host.__sk_keyLayoutChanged = changed;
  return () => {
    observers.delete(observer);
    if (!observers.size && host.__sk_keyLayoutChanged === changed) delete host.__sk_keyLayoutChanged;
  };
}

export function keyScan(code: string): number | undefined {
  return host.__sk_keyScans?.[code];
}
export function keyboardLegend(code: string): string | undefined {
  const raw = host.__sk_keyLegends?.[code];
  if (typeof raw !== "string" || !raw) return undefined;
  const upper = raw.toUpperCase();
  return [...upper].length === [...raw].length ? upper : raw;
}
const oem: Readonly<Record<number, string>> = {
  0xba: "Semicolon",
  0xbb: "Equal",
  0xbc: "Comma",
  0xbd: "Minus",
  0xbe: "Period",
  0xbf: "Slash",
  0xc0: "Backquote",
  0xdb: "BracketLeft",
  0xdc: "Backslash",
  0xdd: "BracketRight",
  0xde: "Quote",
  0xe2: "IntlBackslash",
};
export function capturedPosition({ device, code }: HotkeyButton): string | undefined {
  if (device !== 0) return undefined;
  if ((code & 0xffff0000) === 0x10000 || (code & 0xffff0000) === 0x20000) {
    const scan = code & 0xffff;
    return Object.entries(host.__sk_keyScans ?? {}).find(([, value]) => value === scan)?.[0];
  }
  return (
    host.__sk_keyCodesByVk?.[code] ??
    (code >= 0x41 && code <= 0x5a
      ? "Key" + String.fromCharCode(code)
      : code >= 0x30 && code <= 0x39
        ? "Digit" + String.fromCharCode(code)
        : oem[code])
  );
}
