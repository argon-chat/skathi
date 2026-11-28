import { createApp, defineComponent, h } from "@skathi-native/runtime";
import "./touch.css";

const state = { log: [] as string[] };
(globalThis as { __SK_TEST_TOUCH__?: unknown }).__SK_TEST_TOUCH__ = state;

const where = (e: Event) => (e.currentTarget as Element).id;
const pointer = (e: PointerEvent) =>
  `${e.type}@${where(e)}:${e.pointerType}:${e.pointerId}:${e.isPrimary ? "primary" : "other"}:b${e.buttons}:p${e.pressure.toFixed(2)}:${e.width}x${e.height}`;

const pad = (id: string) => ({
  id,
  onPointerdown: (e: PointerEvent) => state.log.push(pointer(e)),
  onPointermove: (e: PointerEvent) => state.log.push(pointer(e)),
  onPointerup: (e: PointerEvent) => state.log.push(pointer(e)),
  onPointercancel: (e: PointerEvent) => state.log.push(`pointercancel@${where(e)}:${e.pointerId}`),
  onPointerover: (e: PointerEvent) => state.log.push(`pointerover@${where(e)}:${e.pointerId}`),
  onPointerout: (e: PointerEvent) => state.log.push(`pointerout@${where(e)}:${e.pointerId}`),
  onGotpointercapture: (e: PointerEvent) => state.log.push(`got@${where(e)}:${e.pointerId}`),
  onLostpointercapture: (e: PointerEvent) => state.log.push(`lost@${where(e)}:${e.pointerId}`),
  onMousedown: () => state.log.push(`mousedown@${id}`),
  onMouseup: () => state.log.push(`mouseup@${id}`),
  onClick: () => state.log.push(`click@${id}`),
});

const Root = defineComponent({
  setup() {
    return () => h("div", { id: "page" }, [h("div", pad("left")), h("div", pad("right"))]);
  },
});

createApp(Root).mount("#app");
