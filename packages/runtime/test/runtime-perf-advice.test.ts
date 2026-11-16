import { describe, expect, test } from "bun:test";
import { defineComponent, h, nextTick, ref } from "@vue/runtime-core";
import { createRuntime, type MirrorElement } from "./runtime-harness";

const ROWS = 600;

function mountSheet(rt: ReturnType<typeof createRuntime>) {
  const scrollY = ref(0);
  const base = ref(0);
  const width = ref(80);
  let scroller: MirrorElement | null = null;
  const Sheet = defineComponent({
    name: "Sheet",
    setup: () => ({ scrollY, base, width }),
    render() {
      return h("div", null, [
        h("div", { class: "head", style: { marginTop: `${-this.scrollY}px` } }, "head"),
        h(
          "div",
          {
            class: "scroller",
            ref: (el: unknown) => (scroller = el as MirrorElement),
            onScroll: () => (scrollY.value += 24),
          },
          Array.from({ length: ROWS }, (_, i) =>
            h("div", { key: i, class: "row", style: { width: `${this.width}px` } }, String(this.base + i)),
          ),
        ),
      ]);
    },
  });
  rt.createApp(Sheet).mount();
  const scroll = () => rt.host.dispatcher(scroller!.__skId, rt.EV_CODES.scroll, 0, 0, 0, 0);
  return { scroll, base, width };
}

const settle = async () => {
  await nextTick();
  await Promise.resolve();
};

describe("dev-build performance advisories", () => {
  test("a scroll that re-renders a long list to move one header is a wasted re-render, named by its cause", async () => {
    const rt = createRuntime({ perf: true });
    rt.host.perf.visible = () => `${ROWS + 1},40`;
    const sheet = mountSheet(rt);
    await settle();
    sheet.scroll();
    await settle();
    const wasted = rt.host.perf.advisories.find((a) => a.code === "wasted-render");
    expect(wasted?.message).toStartWith(`<Sheet> re-rendered ${ROWS + 3} elements on scroll and 1 of them changed (`);
    expect(wasted?.message).toContain("It was triggered by scrollY.");
    expect(wasted?.message).toContain("position: sticky");
    expect(rt.host.perf.advisories.some((a) => a.code === "long-list")).toBe(true);
  });

  test("a re-render that changes most of what it rendered is not reported", async () => {
    const rt = createRuntime({ perf: true });
    const sheet = mountSheet(rt);
    await settle();
    sheet.base.value = 1000;
    await settle();
    expect(rt.host.perf.advisories).toEqual([]);
  });

  test("more host writes in one frame than the threshold name the component that made them", async () => {
    const rt = createRuntime({ perf: true });
    const sheet = mountSheet(rt);
    await settle();
    sheet.width.value = 90;
    sheet.base.value = 1;
    await settle();
    const writes = rt.host.perf.advisories.find((a) => a.code === "host-writes");
    expect(writes?.message).toStartWith(`${ROWS * 2} style, class, attribute and text writes`);
    expect(writes?.message).toContain(`${ROWS * 2} from <Sheet>`);
  });

  test("a production build measures nothing", async () => {
    const rt = createRuntime({ dev: false, perf: true });
    const sheet = mountSheet(rt);
    await settle();
    sheet.scroll();
    sheet.width.value = 90;
    await settle();
    expect(rt.host.perf.calls).toBe(0);
  });
});
