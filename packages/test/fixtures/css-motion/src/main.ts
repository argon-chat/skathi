import { mountDocument } from "@skathi-native/runtime";
import "./motion.css";

mountDocument(() => {
  const root = document.createElement("main");
  root.id = "motion-root";

  const panelSlot = document.createElement("div");
  panelSlot.id = "panel-slot";

  const tiles = document.createElement("div");
  tiles.id = "tiles";
  for (const id of ["a", "b", "c"]) {
    const tile = document.createElement("div");
    tile.id = id;
    tile.className = "tile";
    tile.textContent = id;
    tiles.appendChild(tile);
  }
  const shadows = document.createElement("div");
  for (const [id, className] of [
    ["shadow", "shadow"],
    ["shadow-inset", "shadow"],
    ["glow", "glow"],
    ["ink", "ink"],
  ] as const) {
    const el = document.createElement("div");
    el.id = id;
    el.className = className;
    el.textContent = id === "ink" ? "ink" : "";
    shadows.appendChild(el);
  }
  root.appendChild(panelSlot);
  root.appendChild(tiles);
  root.appendChild(shadows);
  document.body.appendChild(root);
  return () => root.remove();
});
