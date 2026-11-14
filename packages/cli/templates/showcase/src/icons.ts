// 24×24 stroke paths, drawn with `stroke="currentColor"` so an icon takes its colour from the text.
export const icons = {
  home: "M3 10.5 12 3l9 7.5V21h-6v-6H9v6H3z",
  window: "M3 5h18v14H3z M3 9h18 M6 7h.01",
  tray: "M6 17V10a6 6 0 0 1 12 0v7 M4 17h16 M10 21h4",
  files: "M6 2h8l5 5v15H6z M14 2v5h5 M9 13h6 M9 17h6",
  storage:
    "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3z M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6 M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
  network: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M3 12h18 M12 3c5 5 5 13 0 18 M12 3c-5 5-5 13 0 18",
  media: "M3 5h18v14H3z M3 16l5-5 4 4 3-3 6 6 M16 9h.01",
  audio: "M9 18V5l12-2v13 M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0z M21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
  graphics: "M12 3l9 16H3z M12 9v4",
  motion: "M12 3v4 M12 17v4 M3 12h4 M17 12h4 M6 6l2.5 2.5 M15.5 15.5 18 18 M6 18l2.5-2.5 M15.5 8.5 18 6",
  styles:
    "M12 3a9 9 0 1 0 0 18c1.1 0 2-.9 2-2 0-.5-.2-1-.5-1.3-.3-.4-.5-.8-.5-1.3 0-1.1.9-2 2-2h2.4A4.6 4.6 0 0 0 21 9.8C21 6 17 3 12 3z M7.5 11h.01 M10 7h.01 M15 7.5h.01",
  forms: "M4 7h16v10H4z M8 10v4 M17 4v3 M17 17v3",
  locale: "M4 5h9 M8.5 3v2 M6 5c1 3 3.5 5.5 6.5 6.5 M11 5c-1 3.5-3.5 6.5-7 8 M13 21l4-9 4 9 M14.5 18h5",
  permissions: "M12 3l8 3v6c0 4.5-3.4 8.3-8 9-4.6-.7-8-4.5-8-9V6z M9 12l2 2 4-4",
  system: "M4 5h16v11H4z M8 20h8 M12 16v4 M8 9l2 2-2 2 M13 13h3",
} as const;

export type IconName = keyof typeof icons;
