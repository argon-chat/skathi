import { createApp, defineComponent, h } from "@skathi-native/runtime";
import "./pen.css";

const state = { log: [] as string[] };
(globalThis as { __SK_TEST_PEN__?: unknown }).__SK_TEST_PEN__ = state;

const at = (n: number) => Math.round(n);
const pointer = (e: PointerEvent) =>
  `${e.type}:${e.pointerType}:${e.pointerId}:b${e.button}/${e.buttons}:p${e.pressure.toFixed(2)}:t${e.tiltX},${e.tiltY}`;

const Root = defineComponent({
  setup() {
    const pad = {
      id: "pad",
      onPointerdown: (e: PointerEvent) => {
        state.log.push(pointer(e));
        (e.currentTarget as Element).setPointerCapture(e.pointerId);
      },
      onPointermove: (e: PointerEvent) => {
        const c = e.getCoalescedEvents();
        state.log.push(
          `${pointer(e)}:c${c.length}:${c.map((x) => `${at(x.clientX)}@${x.pressure.toFixed(2)}`).join(",")}`,
        );
      },
      onPointerup: (e: PointerEvent) => state.log.push(pointer(e)),
      onPointerover: (e: PointerEvent) => state.log.push(`pointerover:${e.pointerType}`),
      onPointerout: (e: PointerEvent) => state.log.push(`pointerout:${e.pointerType}`),
      onPointerenter: (e: PointerEvent) => state.log.push(`pointerenter:${e.pointerType}`),
      onPointerleave: (e: PointerEvent) => state.log.push(`pointerleave:${e.pointerType}`),
      onMousedown: (e: MouseEvent) => state.log.push(`mousedown:b${e.button}`),
      onMouseup: (e: MouseEvent) => state.log.push(`mouseup:b${e.button}`),
      onGotpointercapture: (e: PointerEvent) => state.log.push(`got:${e.pointerId}`),
      onLostpointercapture: (e: PointerEvent) => state.log.push(`lost:${e.pointerId}`),
    };
    return () => h("div", { id: "page" }, [h("div", pad), h("div", { id: "away" })]);
  },
});

createApp(Root).mount("#app");
