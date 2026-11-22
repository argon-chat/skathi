import { network } from "skathi:network";

export type Reach = "online" | "no-internet" | "sign-in" | "offline";

function reach(): Reach {
  if (!network.online) return "offline";
  if (network.internet === "portal") return "sign-in";
  return network.internet === "limited" || network.internet === "none" ? "no-internet" : "online";
}

export function watchReach(show: (reach: Reach) => void): () => void {
  const update = () => show(reach());
  update();
  network.addEventListener("change", update);
  return () => network.removeEventListener("change", update);
}
