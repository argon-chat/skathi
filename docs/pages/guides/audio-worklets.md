---
title: Audio worklets
label: Audio worklets
description: Run your own DSP inside a Web Audio graph. AudioWorkletProcessor in JavaScript on the worklet's own thread, or a processor written in Rust that runs in the audio callback.
section: Guides
order: 12
since: "0.7"
status: experimental
---

A Web Audio graph is made of the nodes the platform has. When you need one it does not have (a bit crusher, a synth voice, a level meter with your own ballistics), you write its processing yourself. Skathi has two ways to do that:

- **[`AudioWorkletProcessor`](#a-processor-in-javascript)** in JavaScript, the web's API. It runs on a thread of its own.
- **[A processor in Rust](#a-processor-in-rust)**, for apps with their own host executable. It runs in the audio callback, with no added delay.

Both are experimental: what is described here works, and the limits listed below may move.

## A processor in JavaScript

A processor is a class in a file of its own. The file registers it under a name:

```ts
// src/crush.worklet.ts
class Crush extends AudioWorkletProcessor {
  static get parameterDescriptors() {
    return [{ name: "bits", defaultValue: 8, minValue: 1, maxValue: 16 }];
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][], parameters: Record<string, Float32Array>) {
    const steps = 2 ** parameters.bits![0]!;
    const input = inputs[0]!;
    const output = outputs[0]!;
    for (let c = 0; c < output.length; c++) {
      const from = input[c]!;
      const to = output[c]!;
      for (let i = 0; i < to.length; i++) to[i] = Math.round(from[i]! * steps) / steps;
    }
    return true;
  }
}

registerProcessor("crush", Crush);
```

The page loads the file into the context's worklet, then makes nodes by that name:

```ts
import crushUrl from "./crush.worklet.ts?worklet";

const context = new AudioContext();
await context.audioWorklet.addModule(crushUrl);

const crush = new AudioWorkletNode(context, "crush", { parameterData: { bits: 6 } });
source.connect(crush).connect(context.destination);
crush.parameters.get("bits")!.setValueAtTime(4, context.currentTime + 1);
```

`?worklet` makes the build bundle the file, with everything it imports, into one script, and gives the page its URL. `context.audioWorklet.addModule(new URL("./crush.worklet.ts", import.meta.url))`, the form browsers and Vite read, does the same.

The node and its processor talk through their ports:

```ts
// In the processor
this.port.onmessage = (event) => {
  this.port.postMessage({ peak: this.peak, asked: event.data });
};

// On the page
crush.port.onmessage = (event) => console.log(event.data.peak);
crush.port.postMessage("peak?");
```

A processor that throws stops, its node goes silent, and the node gets a `processorerror` event whose `message` is the error's.

### What a processor file has

The worklet is not a page, and not a worker. Its global scope has the JavaScript language and these:

| Name                                            | What it is                                                                  |
| ----------------------------------------------- | --------------------------------------------------------------------------- |
| `AudioWorkletProcessor`                         | The class to extend. `this.port` is the processor's end of the node's port. |
| `registerProcessor(name, class)`                | Registers a processor. A name can be registered once in a context.          |
| `sampleRate`                                    | The context's sample rate.                                                  |
| `currentFrame`, `currentTime`                   | Where the block being processed starts, in frames and in seconds.           |
| `console.log`, `info`, `debug`, `warn`, `error` | Print to the host's standard error, with an `[audio worklet]` prefix.       |

There is no `fetch`, no timers, no DOM and no `import()`. An `import` at the top of the file works because the build puts the imported code into the file; what the scope runs is one script. Inside a worklet file, import code only: `?raw`, `import.meta.glob` and asset imports fail the build. Send a processor its data (a wavetable, an impulse response) through `processorOptions` or its port.

### How it runs

Each `AudioContext` that loads a module gets one worklet thread, and that thread has a JavaScript VM of its own, apart from the app's. The engine cannot yet run one VM on two threads, so the worklet's code and data live in a second VM, and everything that crosses between the page and a processor is copied.

This is a stopgap. When the engine can run several threads in one VM, a worklet will be a thread of the app's VM, without the second copy of the engine's state, and the API on this page stays as it is. Web Workers are a different matter and are not planned alongside: a worklet's scope is small and fixed, which is what makes it possible to host it this way.

What follows from the design today:

- **Delay.** The audio callback does not wait for JavaScript. A processor renders slightly ahead of the callback, and a node's output reaches the graph at least 256 frames (5 ms at 48 kHz) later than in a browser. If the processor cannot keep up, its node plays silence for the missing frames and the delay grows by that much; the rest of the graph is not held up.
- **No JIT in an AOT build.** An app built with [`--aot`](/guides/aot/) has no JIT, and a worklet is not part of the app's image, so its `process()` is interpreted. A filter or a meter is fine; a heavy synth is a case for a processor in Rust.
- **Memory.** A worklet's VM has a heap of its own. Closing the context stops the thread, and the VM's memory is returned when the app exits.
- **A hidden window.** The worklet thread is not throttled when the window is hidden or minimised. Messages to the page are delivered when the page next runs.

### Limits

- A node has at most one input and one output. The input always has two channels; the output has two, or one with `outputChannelCount: [1]`. `numberOfInputs: 0` makes a source and `numberOfOutputs: 0` an analyser.
- A parameter has one value for each block: `parameters.name` is a `Float32Array` of length 1, and automation moves it once per frame of the page, as for every `AudioParam` in Skathi.
- A message is copied, never transferred. It can hold numbers, strings, booleans, `null`, `undefined`, arrays, plain objects, `ArrayBuffer`s and typed arrays; anything else throws a `DataCloneError`. The same holds for `processorOptions`.
- Inside a processor, a port has `postMessage` and `onmessage`, and no `addEventListener`. On the page, `node.port` is an `EventTarget` with both, named `AudioWorkletPort` rather than `MessagePort`.
- A processor's `process()` returning `false` ends a processor that has no input. One with an input runs for as long as its node is alive.
- `OfflineAudioContext` has no worklet: its `addModule()` rejects with `NotSupportedError`.
- A processor can declare up to 32 parameters.

## A processor in Rust

An app with [its own host executable](/embedding/overview/) can write a processor in Rust, with the `audio` feature of the `skathi` crate on, as it is by default. It runs in the audio callback itself, so it adds no delay and does not depend on the JIT, and in return it must behave like any code on an audio thread: no allocation, no locks, no waiting.

```rust
use skathi::audio::{AudioProcessor, ProcessIo, ProcessorSpec};

struct Crush;

impl AudioProcessor for Crush {
    fn process(&mut self, io: &mut ProcessIo<'_>) {
        let steps = 2f32.powf(io.params[0]);
        let mut peak = 0f32;
        for (output, input) in io.outputs.iter_mut().zip(io.inputs) {
            for (to, from) in output.iter_mut().zip(input) {
                *to = (from * steps).round() / steps;
                peak = peak.max(to.abs());
            }
        }
        io.post(&[peak]);
    }
}

fn main() {
    let builder = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .audio_processor(
            "crush",
            ProcessorSpec::new(|_sample_rate| Box::new(Crush)).param("bits", 8.0, 1.0, 16.0),
        );
    // …
}
```

The app makes the node by name:

```ts
const crush = context.loadNativeAudioWorklet("crush", { parameterData: { bits: 6 } });
source.connect(crush).connect(context.destination);
crush.parameters.get("bits").value = 4;
crush.port.onmessage = (event) => meter.set(event.data[0]);
```

- The closure given to `ProcessorSpec::new` makes one processor for each node. It is called off the audio thread, so it may allocate.
- `io.inputs` and `io.outputs` are two channels of `io.frames()` samples. A mono source arrives on both.
- `io.params` holds the parameters in the order `param()` declared them, one value for the block.
- `io.post(&[…])` leaves up to 16 numbers for the page, which gets them as a `Float32Array` in a `message` event on `node.port`. Only the latest message is kept until the page reads it, so post a state (a level, a flag), not a stream.
- The page cannot post to a native processor: drive it with its parameters.

`loadNativeAudioWorklet` also has four processors built in: `peak-meter-stereo`, `peak-meter-mono`, `stereo-to-mono` and `input-gate`.
