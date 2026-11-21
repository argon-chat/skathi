import { LEVEL, describe } from "./level";

class Tone extends AudioWorkletProcessor {
  constructor() {
    super();
    this.port.onmessage = (event) => {
      this.port.postMessage({ answer: describe("tone", String(event.data)), rate: sampleRate });
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    for (const channel of outputs[0] ?? []) channel.fill(LEVEL);
    return true;
  }
}

registerProcessor("tone", Tone);
