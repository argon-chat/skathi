// The tray icon and the global hotkey outlive the page that turns them on, so their state lives here.
import { appHotkeys, appTray, appWindow, type HotkeyChord, type TrayPixels } from "@skathi-native/runtime";
import { defineStore } from "pinia";
import { ref, watch } from "vue";
import { productName } from "../product";
import { useCounter } from "./counter";

const CTRL = 0x11;
const SHIFT = 0x10;
const KEY_K = 0x4b;
export const DEFAULT_CHORD: HotkeyChord = {
  buttons: [
    { device: 0, code: CTRL },
    { device: 0, code: SHIFT },
    { device: 0, code: KEY_K },
  ],
};

const KEY_NAMES: Record<number, string> = {
  0x10: "Shift",
  0xa0: "Shift",
  0xa1: "Shift",
  0x11: "Ctrl",
  0xa2: "Ctrl",
  0xa3: "Ctrl",
  0x12: "Alt",
  0xa4: "Alt",
  0xa5: "Alt",
  0x5b: "Win",
  0x5c: "Win",
  0x20: "Space",
  0x0d: "Enter",
  0x09: "Tab",
};

/** Virtual-key codes on every platform: letters and digits are their ASCII codes, F1 is 0x70. */
export function chordLabel(chord: HotkeyChord): string {
  return chord.buttons
    .map(({ device, code }) => {
      if (device === 1) return `Mouse ${code}`;
      if ((code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a)) return String.fromCharCode(code);
      if (code >= 0x70 && code <= 0x87) return `F${code - 0x6f}`;
      return KEY_NAMES[code] ?? `0x${code.toString(16)}`;
    })
    .join(" + ");
}

function trayIcon(): TrayPixels {
  const size = 32;
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dx = Math.max(0, Math.abs(x - 15.5) - 8);
      const dy = Math.max(0, Math.abs(y - 15.5) - 8);
      const t = (x + y) / (2 * size);
      const o = (y * size + x) * 4;
      data[o] = Math.round(0x8b - 0x1e * t);
      data[o + 1] = Math.round(0x9d - 0x21 * t);
      data[o + 2] = 0xff;
      data[o + 3] = dx * dx + dy * dy <= 64 ? 255 : 0;
    }
  }
  return { width: size, height: size, data };
}

export const useDesktop = defineStore("desktop", () => {
  const counter = useCounter();
  const trayShown = ref(false);
  const chord = ref<HotkeyChord | null>(null);
  const presses = ref(0);

  function showTray(on: boolean) {
    trayShown.value = on;
    if (!on) {
      appTray.set(null);
      return;
    }
    appTray.set({
      icon: trayIcon(),
      tooltip: productName,
      onClick: () => appWindow.show(),
      menu: [
        { label: `Open ${productName}`, default: true, onSelect: () => appWindow.show() },
        { label: `Count up (${counter.count})`, onSelect: counter.increment },
        { label: "Always on top", checked: appWindow.alwaysOnTop, onSelect: toggleOnTop },
        { type: "separator" },
        { role: "quit" },
      ],
    });
  }

  function toggleOnTop() {
    appWindow.alwaysOnTop = !appWindow.alwaysOnTop;
    showTray(true);
  }

  // The tray has no reactive model: a new menu is set whenever what it shows changes.
  watch(
    () => counter.count,
    () => trayShown.value && showTray(true),
  );

  let unlisten: (() => void) | null = null;
  function bindHotkey(next: HotkeyChord | null) {
    appHotkeys.setBindings(next ? [{ id: "count-up", chord: next, triggerCooldownMs: 150 }] : []);
    chord.value = next;
    unlisten ??= appHotkeys.onFired((event) => {
      if (event.id !== "count-up" || event.phase !== "down") return;
      presses.value++;
      counter.increment();
    });
  }

  return { trayShown, chord, presses, showTray, bindHotkey };
});
