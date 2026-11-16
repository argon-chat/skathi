import { watch } from "vue";
import { appLaunch, appTray, appWindow } from "@skathi-native/runtime";
import { background, draft, go, saveDraft } from "./store";

function onCloseRequested(e: Event) {
  if (background.keepRunning) {
    e.preventDefault();
    appWindow.hide();
    return;
  }
  if (draft.text === draft.saved) return;
  e.preventDefault();
  if (appWindow.isMinimized()) appWindow.restore();
  go("closing/unsaved-changes");
  draft.asking = true;
}

function bringBack(source: string) {
  appWindow.show();
  background.returns++;
  background.lastReturn = source;
}

function trayIcon(): Uint8Array {
  const size = 16;
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const d = Math.hypot(x + 0.5 - 8, y + 0.5 - 8);
      const a = Math.max(0, Math.min(1, 7 - d));
      const i = (y * size + x) * 4;
      data.set([0x8a, 0x44, 0xa3, Math.round(a * 255)], i);
    }
  }
  return data;
}

export function installClosing(): void {
  appWindow.addEventListener("close-requested", onCloseRequested);
  appLaunch.addEventListener("launch", () => bringBack("a second launch"));

  watch(
    () => background.keepRunning,
    (on) => {
      if (!appTray.available) return;
      appTray.set(
        on
          ? {
              icon: { width: 16, height: 16, data: trayIcon() },
              tooltip: "Windows (running in the background)",
              onClick: () => bringBack("the tray icon"),
              menu: [
                { label: "Open", default: true, onSelect: () => bringBack("the tray menu") },
                { type: "separator" },
                { label: "Quit", onSelect: () => appWindow.close() },
              ],
            }
          : null,
      );
    },
  );
}

export function answerClose(choice: "save" | "discard" | "cancel"): void {
  draft.asking = false;
  if (choice === "cancel") return;
  if (choice === "save") saveDraft();
  appWindow.close();
}
