import { appHost, createApp, h, ref } from "@skathi-native/runtime";

const events: string[] = [];
const hiddenShown = ref(false);
Object.assign(globalThis, {
  surfaceEvents: events,
  showHidden: () => (hiddenShown.value = true),
  hostCall: (name: string, ...args: unknown[]) => appHost.call(name, ...args),
});

const at = (left: number, top: number, width: number, height: number, extra = "") =>
  `position: absolute; left: ${left}px; top: ${top}px; width: ${width}px; height: ${height}px; ${extra}`;

const log = (id: string) => ({
  onSurfaceresize: (e: SkSurfaceEvent) => events.push(`${id}:resize:${e.width}x${e.height}@${e.devicePixelRatio}`),
  onSurfaceready: (e: SkSurfaceEvent) => events.push(`${id}:ready:${e.width}x${e.height}`),
  onSurfacelost: (e: SkSurfaceEvent) => events.push(`${id}:lost:${e.reason}`),
});

createApp({
  render: () =>
    h("div", { id: "root", style: "position: absolute; inset: 0; background: #ffffff" }, [
      h("sk-surface", { id: "fill", name: "fill", style: at(20, 20, 160, 100, "object-fit: fill"), ...log("fill") }),
      h("div", { id: "over", style: at(20, 20, 40, 40, "background: #ffffff; z-index: 1") }),
      h("sk-surface", { id: "live", name: "live", style: at(420, 20, 120, 80), ...log("live") }),
      h("sk-surface", { id: "contain", name: "contain", style: at(200, 20, 200, 100), ...log("contain") }),
      h("sk-surface", {
        id: "round",
        name: "round",
        style: at(20, 140, 100, 100, "border-radius: 50%; object-fit: cover"),
        ...log("round"),
      }),
      h("sk-surface", {
        id: "hidden",
        name: "hidden",
        style: at(200, 140, 80, 80, hiddenShown.value ? "" : "display: none"),
        ...log("hidden"),
      }),
      ...["d3d12", "d3d12-shared", "d3d11-fence"].map((name, i) =>
        h("sk-surface", { id: name, name, style: at(20 + i * 120, 260, 100, 80, "object-fit: fill"), ...log(name) }),
      ),
      ...["metal", "iosurface", "mach-port", "cv-bgra", "cv-nv12"].map((name, i) =>
        h("sk-surface", { id: name, name, style: at(20 + i * 120, 360, 100, 80, "object-fit: fill"), ...log(name) }),
      ),
    ]),
}).mount();
