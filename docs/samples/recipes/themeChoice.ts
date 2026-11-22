import { appWindow } from "skathi:app";

type Theme = "system" | "light" | "dark";

const KEY = "theme";
const isTheme = (value: unknown): value is Theme => value === "system" || value === "light" || value === "dark";

export function restoreTheme(): Theme {
  const saved = localStorage.getItem(KEY);
  const theme = isTheme(saved) ? saved : "system";
  appWindow.theme = theme;
  return theme;
}

export function chooseTheme(theme: Theme): void {
  appWindow.theme = theme;
  localStorage.setItem(KEY, theme);
}
