import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

function callbacks() {
  let next = 1;
  const frames = new Map<number, (...args: unknown[]) => unknown>();
  const timers = new Map<number, { callback: (...args: unknown[]) => unknown; args: unknown[] }>();
  return {
    frames,
    timers,
    globals: {
      requestAnimationFrame(callback: (...args: unknown[]) => unknown) {
        const id = next++;
        frames.set(id, callback);
        return id;
      },
      cancelAnimationFrame(id: number) {
        frames.delete(id);
      },
      setTimeout(callback: (...args: unknown[]) => unknown, _delay: unknown, ...args: unknown[]) {
        const id = next++;
        timers.set(id, { callback, args });
        return id;
      },
      clearTimeout(id: number) {
        timers.delete(id);
      },
    },
  };
}

describe("original callback profiling", () => {
  test("rAF registered before recording reports the actual callback and preserves receiver/arguments/result", () => {
    const scheduler = callbacks();
    let recording = false;
    const seen: unknown[] = [],
      ended: number[] = [];
    const rt = createRuntime({
      globals: {
        ...scheduler.globals,
        __vn_trace_callback_begin(callback: unknown) {
          if (!recording) return 0;
          seen.push(callback);
          return 17;
        },
        __vn_trace_callback_end(token: number) {
          ended.push(token);
        },
      },
    });
    const receiver = {};
    let actualThis: unknown, actualTimestamp: unknown;
    function profiledFrame(this: unknown, timestamp: unknown) {
      actualThis = this;
      actualTimestamp = timestamp;
      return 31;
    }
    const id = (rt.global.requestAnimationFrame as (cb: unknown) => number)(profiledFrame);
    recording = true;
    expect(scheduler.frames.get(id)!.call(receiver, 123.5)).toBe(31);
    expect(seen).toEqual([profiledFrame]);
    expect(ended).toEqual([17]);
    expect(actualThis).toBe(receiver);
    expect(actualTimestamp).toBe(123.5);
  });

  test("active timer callbacks end in finally while their original exception is preserved", () => {
    const scheduler = callbacks();
    let recording = false;
    const seen: unknown[] = [],
      ended: number[] = [];
    const rt = createRuntime({
      globals: {
        ...scheduler.globals,
        __vn_trace_callback_begin(callback: unknown) {
          if (!recording) return 0;
          seen.push(callback);
          return 23;
        },
        __vn_trace_callback_end(token: number) {
          ended.push(token);
        },
      },
    });
    const original = new Error("original");
    function profiledTimer(value: unknown) {
      expect(value).toBe("argument");
      recording = false;
      throw original;
    }
    const id = (rt.global.setTimeout as (...args: unknown[]) => number)(profiledTimer, 0, "argument");
    recording = true;
    const timer = scheduler.timers.get(id)!;
    let caught: unknown;
    try {
      timer.callback(...timer.args);
    } catch (error) {
      caught = error;
    }
    expect(caught).toBe(original);
    expect(seen).toEqual([profiledTimer]);
    expect(ended).toEqual([23]);
  });

  test("inactive controls return directly without an end call", () => {
    const scheduler = callbacks();
    let begins = 0,
      ends = 0;
    const rt = createRuntime({
      globals: {
        ...scheduler.globals,
        __vn_trace_callback_begin() {
          begins++;
          return 0;
        },
        __vn_trace_callback_end() {
          ends++;
        },
      },
    });
    const id = (rt.global.setTimeout as (...args: unknown[]) => number)((a: unknown) => a, 0, 49);
    const timer = scheduler.timers.get(id)!;
    expect(timer.callback(...timer.args)).toBe(49);
    expect(begins).toBe(1);
    expect(ends).toBe(0);
  });

  test("the mirror event walk passes the original user listener to the hook", () => {
    const seen: unknown[] = [],
      ended: number[] = [];
    const rt = createRuntime({
      globals: {
        __vn_trace_callback_begin(callback: unknown) {
          seen.push(callback);
          return 29;
        },
        __vn_trace_callback_end(token: number) {
          ended.push(token);
        },
      },
    });
    const node = rt.nodeOps.createElement("button");
    let calls = 0;
    function profiledClick() {
      calls++;
    }
    rt.patchProp(node, "onClick", null, profiledClick);
    rt.host.dispatcher(node.__vnId, rt.EV_CODES.click, 0, 0, 0, 0, "");
    expect(calls).toBe(1);
    expect(seen).toContain(profiledClick);
    expect(ended.length).toBe(seen.length);
  });
});
