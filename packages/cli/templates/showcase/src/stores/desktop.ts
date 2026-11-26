// The tray icon and the global hotkey outlive the page that turns them on, so their state lives here.
import { defineKeymap, type SkKeyMap, type TrayPixels } from "@skathi-native/runtime";
import { appTray, appWindow } from "skathi:app";
import { defineStore } from "pinia";
import { onScopeDispose, ref, shallowRef, watch } from "vue";
import { productName } from "../product";
import { useCounter } from "./counter";

export const appCommand = defineKeymap({ keys: ["Mod", { code: "KeyK" }], scope: "app", suppress: true });
export const globalCommand = defineKeymap({ keys: "Control+Shift+KeyK", scope: "global", triggerCooldownMs: 150 });

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
  const boundMap = shallowRef<SkKeyMap | null>(null);
  const appPresses = ref(0);
  onScopeDispose(appCommand.bind(() => appPresses.value++));
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

  let unbind: (() => void) | undefined;
  function bindHotkey(next: SkKeyMap | null) {
    const stop = next?.bind(() => {
      presses.value++;
      counter.increment();
    });
    unbind?.();
    unbind = stop;
    boundMap.value = next;
  }
  onScopeDispose(() => unbind?.());

  return { trayShown, boundMap, appPresses, presses, showTray, bindHotkey };
});
