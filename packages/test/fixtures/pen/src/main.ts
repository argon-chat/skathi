import { appHaptics, createApp, defineComponent, h } from "@skathi-native/runtime";
import "./pen.css";

interface Move {
  steps: number[];
  beforeEvent: boolean;
  recent: boolean;
  ahead: number[];
  forward: boolean;
}
const state = { haptics: appHaptics, log: [] as string[], moves: [] as Move[] };
(globalThis as { __SK_TEST_PEN__?: unknown }).__SK_TEST_PEN__ = state;

const at = (n: number) => Math.round(n);
const pointer = (e: PointerEvent) =>
  `${e.type}:${e.pointerType}:${e.pointerId}:b${e.button}/${e.buttons}:p${e.pressure.toFixed(2)}:t${e.tiltX},${e.tiltY}`;

/** A move's reading times against each other, the move and `performance.now()`, and its predictions. */
function timing(e: PointerEvent, c: PointerEvent[]): Move {
  const p = e.getPredictedEvents();
  const last = c[c.length - 1]!;
  const age = performance.now() - last.timeStamp;
  return {
    steps: c.slice(1).map((x, i) => at(x.timeStamp - c[i]!.timeStamp)),
    beforeEvent: c.every((x) => x.timeStamp <= e.timeStamp),
    recent: age >= 0 && age < 1000,
    ahead: p.map((x) => at(x.timeStamp - last.timeStamp)),
    forward: p.every((x, i) => x.clientX > (i ? p[i - 1]! : last).clientX),
  };
}

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
        state.moves.push(timing(e, c));
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
