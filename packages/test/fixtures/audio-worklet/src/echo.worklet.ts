import { describe } from "./level";

registerProcessor(
  "echo",
  class extends AudioWorkletProcessor {
    constructor() {
      super();
      this.port.onmessage = (event) => this.port.postMessage({ answer: describe("echo", String(event.data)) });
    }

    process(): boolean {
      return true;
    }
  },
);
