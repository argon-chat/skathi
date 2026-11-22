import { appLaunch, appWindow } from "skathi:app";
import type { SkLaunchEvent } from "@skathi-native/runtime";

const SCHEME = "notes";

function route(link: string | null, go: (url: URL) => void): void {
  if (!link) return;
  let url: URL;
  try {
    url = new URL(link);
  } catch {
    return;
  }
  if (url.protocol === `${SCHEME}:`) go(url);
}

export function handleDeepLinks(go: (url: URL) => void): void {
  if (!appLaunch.isProtocolRegistered(SCHEME)) {
    try {
      appLaunch.registerProtocol(SCHEME);
    } catch {
      // Not on this platform yet: the app works without links.
    }
  }
  route(appLaunch.url, go);
  appLaunch.addEventListener("launch", (event) => {
    route((event as SkLaunchEvent).url, go);
    appWindow.show();
  });
}
