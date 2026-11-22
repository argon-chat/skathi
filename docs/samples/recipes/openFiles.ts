import { appLaunch, appWindow } from "skathi:app";
import type { SkLaunchEvent } from "@skathi-native/runtime";

export function openLaunchedFiles(open: (path: string) => void): void {
  appLaunch.paths.forEach(open);
  appLaunch.addEventListener("launch", (event) => {
    (event as SkLaunchEvent).paths.forEach(open);
    appWindow.show();
  });
}
