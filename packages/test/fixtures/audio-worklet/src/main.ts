import { createApp, h } from "@skathi-native/runtime";
import toneUrl from "./tone.worklet.ts?worklet";

const fixture = {
  toneUrl,
  async load(ctx: AudioContext): Promise<void> {
    await ctx.audioWorklet.addModule(toneUrl);
    await ctx.audioWorklet.addModule(new URL("./echo.worklet.ts", import.meta.url));
  },
  ask(node: AudioWorkletNode, question: string): Promise<unknown> {
    return new Promise((resolve) => {
      node.port.onmessage = (event) => resolve(event.data);
      node.port.postMessage(question);
    });
  },
};
(globalThis as unknown as { __SK_TEST_WORKLET__: typeof fixture }).__SK_TEST_WORKLET__ = fixture;

createApp({ render: () => h("div", { id: "root" }, "audio worklet") }).mount("#app");
