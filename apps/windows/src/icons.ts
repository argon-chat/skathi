// 24×24 stroke paths, drawn with `stroke="currentColor"`.
export const icons = {
  window: "M3 5h18v14H3z M3 9h18",
  pin: "M9 3h6 M10 3v6l-3 4h10l-3-4V3 M12 13v8",
  theme: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 3v18 M12 7h5 M12 11h8 M12 15h7",
  fullscreen: "M4 9V4h5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5",
  drag: "M12 3v18 M3 12h18 M9 6l3-3 3 3 M9 18l3 3 3-3 M6 9l-3 3 3 3 M18 9l3 3-3 3",
  tabs: "M3 20V8h6V5h12v15z M9 8h12",
  resize: "M4 4h10v10H4z M18 10v10H8 M15 17h3v-3",
  palette:
    "M12 3a9 9 0 1 0 0 18c1.1 0 2-.9 2-2 0-.5-.2-1-.5-1.3-.3-.4-.5-.8-.5-1.3 0-1.1.9-2 2-2h2.4A4.6 4.6 0 0 0 21 9.8C21 6 17 3 12 3z M7.5 11h.01 M10 7h.01 M15 7.5h.01",
  follow: "M4 5h10v14H4z M7 9h4 M7 13h4 M17 11l4-3v8z",
  overlay: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 7v5l3 2",
  reattach: "M14 4h6v6 M20 4l-8 8 M18 14v6H4V6h6",
  unsaved: "M5 3h11l3 3v15H5z M8 3v5h7V3 M9 14h6 M9 17h4",
  background: "M4 7h16v12H4z M8 3h8 M12 11v4 M10 13h4",
  minimize: "M5 12h14",
  maximize: "M5 5h14v14H5z",
  restore: "M8 4h12v12 M4 8h12v12H4z",
  toggle: "M4 9h12 M13 6l3 3-3 3 M20 15H8 M11 12l-3 3 3 3",
  close: "M6 6l12 12 M18 6 6 18",
  check: "M5 12l5 5 9-10",
  plus: "M12 5v14 M5 12h14",
  doc: "M6 3h8l4 4v14H6z M14 3v4h4",
  image: "M3 5h18v14H3z M3 16l5-5 4 4 3-3 6 6",
  archive: "M3 5h18v4H3z M5 9v10h14V9 M10 13h4",
  sheet: "M4 4h16v16H4z M4 10h16 M4 15h16 M10 4v16",
  music: "M9 18V5l12-2v13 M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0z M21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
} as const;

export type IconName = keyof typeof icons;
