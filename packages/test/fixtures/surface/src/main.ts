import { createApp, h, ref } from "@skathi-native/runtime";
import { appHost } from "skathi:app";

const events: string[] = [];
const hiddenShown = ref(false);
Object.assign(globalThis, {
  surfaceEvents: events,
  showHidden: () => (hiddenShown.value = true),
  hostCall: (name: string, ...args: unknown[]) => appHost.call(name, ...args),
});

// Half-float surfaces: the CPU path, then the Windows and the macOS GPU paths.
const HALF = [
  "cpu-rgba16f",
  "texture-rgba16f",
  "shared-rgba16f",
  "d3d12-rgba16f",
  "d3d12-shared-rgba16f",
  "metal-rgba16f",
  "iosurface-rgba16f",
  "cv-rgha",
];

const at = (left: number, top: number, width: number, height: number, extra = "") =>
  `position: absolute; left: ${left}px; top: ${top}px; width: ${width}px; height: ${height}px; ${extra}`;

const log = (id: string) => ({
  onSurfaceresize: (e: SkSurfaceEvent) => events.push(`${id}:resize:${e.width}x${e.height}@${e.devicePixelRatio}`),
  onSurfaceready: (e: SkSurfaceEvent) => events.push(`${id}:ready:${e.width}x${e.height}`),
  onSurfacelost: (e: SkSurfaceEvent) => events.push(`${id}:lost:${e.reason}`),
  onSurfacedisplaychange: (e: SkSurfaceDisplayEvent) =>
    events.push(`${id}:display:${e.headroom}:${e.refreshRate}:${e.hdr}:${e.outputHdr}`),
});

createApp({
  render: () =>
    h("div", { id: "root", style: "position: absolute; inset: 0; background: #ffffff" }, [
      h("sk-surface", { id: "fill", name: "fill", style: at(20, 20, 160, 100, "object-fit: fill"), ...log("fill") }),
      h("div", { id: "over", style: at(20, 20, 40, 40, "background: #ffffff; z-index: 1") }),
      h("sk-surface", { id: "live", name: "live", style: at(420, 20, 120, 80), ...log("live") }),
      h("sk-surface", { id: "feedback", name: "feedback", style: at(560, 20, 120, 80), ...log("feedback") }),
      h("sk-surface", { id: "ring", name: "ring", style: at(560, 140, 120, 80), ...log("ring") }),
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
      ...HALF.map((name, i) =>
        h("sk-surface", { id: name, name, style: at(20 + i * 86, 460, 80, 60, "object-fit: fill"), ...log(name) }),
      ),
    ]),
}).mount();
