import { appTray, appWindow } from "skathi:app";

export function showUnread(count: number): void {
  if (count > 0) void navigator.setAppBadge(count);
  else void navigator.clearAppBadge();

  if (!appTray.available) return;
  appTray.set({
    tooltip: count > 0 ? `${count} unread` : "No unread messages",
    onClick: () => appWindow.show(),
  });
}
