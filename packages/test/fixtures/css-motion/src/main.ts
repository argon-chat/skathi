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
  root.appendChild(panelSlot);
  root.appendChild(tiles);
  document.body.appendChild(root);
  return () => root.remove();
});
