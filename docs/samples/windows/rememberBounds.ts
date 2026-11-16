import { appWindow } from "@skathi-native/runtime";

interface Saved {
  x: number;
  y: number;
  width: number;
  height: number;
  maximized: boolean;
}

const KEY = "window-bounds";
const isNum = (n: unknown): n is number => typeof n === "number" && Number.isFinite(n);

function load(): Saved | null {
  try {
    const s: Partial<Record<keyof Saved, unknown>> = JSON.parse(localStorage.getItem(KEY) ?? "null") ?? {};
    const { x, y, width, height } = s;
    if (!isNum(x) || !isNum(y) || !isNum(width) || !isNum(height)) return null;
    return { x, y, width, height, maximized: s.maximized === true };
  } catch {
    return null;
  }
}

export function rememberBounds(): void {
  const saved = load();
  if (saved) {
    appWindow.setBounds({ x: saved.x, y: saved.y, width: saved.width, height: saved.height });
    if (saved.maximized) appWindow.maximize();
  }

  let normal = saved ?? appWindow.getBounds();
  let maximized = saved?.maximized ?? false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const write = () => localStorage.setItem(KEY, JSON.stringify({ ...normal, maximized }));
  const record = () => {
    if (appWindow.isMinimized() || appWindow.isFullscreen()) return;
    maximized = appWindow.isMaximized();
    if (!maximized) normal = appWindow.getBounds();
    clearTimeout(timer);
    timer = setTimeout(write, 300);
  };

  appWindow.addEventListener("move", record);
  appWindow.addEventListener("maximize", record);
  appWindow.addEventListener("unmaximize", record);
  addEventListener("resize", record);
  addEventListener("pagehide", write);
}
