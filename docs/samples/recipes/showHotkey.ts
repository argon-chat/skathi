import { appHotkeys, appWindow } from "skathi:app";

const CTRL = 0xa2;
const SHIFT = 0xa0;
const SPACE = 0x20;

export function showOnHotkey(): () => void {
  if (!appHotkeys.available) return () => {};
  appHotkeys.setBindings([
    {
      id: "show",
      chord: {
        buttons: [
          { device: 0, code: CTRL },
          { device: 0, code: SHIFT },
          { device: 0, code: SPACE },
        ],
      },
    },
  ]);
  return appHotkeys.onFired((event) => {
    if (event.id === "show" && event.phase === "down") appWindow.show();
  });
}
