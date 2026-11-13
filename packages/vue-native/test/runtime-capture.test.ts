import { expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

test("desktop source list crosses the asynchronous bridge as an array", async () => {
  const rt = createRuntime();
  const sources = [
    {
      id: "screen:123",
      name: "Display",
      thumbnailDataUrl: "data:image/png;base64,x",
      appIconDataUrl: null,
      displayId: "123",
    },
  ];
  const pending = rt.appCapture.getSources(["screen"], { withIcons: false });
  expect(rt.host.desktop.calls.at(-1)).toEqual({ op: "screenSources", args: [["screen"], { withIcons: false }] });
  rt.host.desktop.settle(1, true, sources);
  expect(await pending).toEqual(sources);
});

test("invalid source types and missing native capture reject explicitly", async () => {
  const rt = createRuntime();
  await expect(rt.appCapture.getSources(["tab"])).rejects.toThrow(TypeError);
  expect(rt.host.desktop.calls).toEqual([]);
  await expect(createRuntime({ desktop: false }).appCapture.getSources()).rejects.toMatchObject({
    name: "NotSupportedError",
  });
  const pending = rt.appCapture.getSources();
  rt.host.desktop.settle(1, true, {});
  await expect(pending).rejects.toThrow("invalid source list");
});

test("a capture source's refusal hint is the platform's, by permission name", () => {
  const rt = createRuntime();
  expect(rt.appCapture.permissionHint("screen_share")).toBe("");
  rt.host.permissions.hints.set("display-capture", "Turn on Argon in System Settings.");
  expect(rt.appCapture.permissionHint("screen_share")).toBe("Turn on Argon in System Settings.");
  expect(rt.appCapture.permissionHint("camera")).toBe("");
});

test("a capture source's permission is read without asking, with whose denial it is", () => {
  const rt = createRuntime();
  rt.host.permissions.states.set("microphone", "denied");
  rt.host.permissions.states.set("camera", "denied");
  rt.host.permissions.states.set("display-capture", "granted");
  rt.host.permissions.bySystem.add("camera");
  expect(rt.appCapture.permission("microphone")).toEqual({ state: "denied", canRequest: true, deniedBySystem: false });
  expect(rt.appCapture.permission("camera")).toEqual({ state: "denied", canRequest: true, deniedBySystem: true });
  expect(rt.appCapture.permission("screen_share")).toEqual({
    state: "granted",
    canRequest: false,
    deniedBySystem: false,
  });
  expect(rt.host.permissions.requests).toEqual([]);
  expect(() => rt.appCapture.permission("tab" as never)).toThrow(TypeError);
});

test("the microphone is a capture source to ask for and to hint at", async () => {
  const rt = createRuntime();
  rt.host.permissions.states.set("microphone", "granted");
  expect(await rt.appCapture.requestPermission("microphone")).toBe("granted");
  expect(rt.host.permissions.requests).toEqual(["microphone"]);
  rt.host.permissions.hints.set("microphone", "Turn on Argon in System Settings › Privacy & Security › Microphone.");
  expect(rt.appCapture.permissionHint("microphone")).toContain("Microphone");
  await expect(rt.appCapture.requestPermission("speaker" as never)).rejects.toThrow(TypeError);
});

test("screen selection keeps the exact native source ID and rejects invalid IDs", () => {
  const rt = createRuntime();
  expect(rt.appCapture.setPendingSource("window:456", true)).toBe(true);
  expect(rt.global.__VN_SCREEN_SOURCE__).toEqual({ sourceId: "window:456", includeAudio: true });
  expect(() => rt.appCapture.setPendingSource("window:NaN")).toThrow(TypeError);
});

test("video srcObject and Vue binding retain the stream and route native frames", () => {
  const rt = createRuntime();
  const el = rt.nodeOps.createElement("video");
  const stream = { getVideoTracks: () => [{ __vnVideoSrc: "vn-stream://42" }] };
  expect(el.srcObject).toBe(null);
  rt.patchProp(el, "srcObject", null, stream);
  expect(el.srcObject).toBe(stream);
  expect(el.getAttribute("src")).toBe("vn-stream://42");
  expect(el.getAttribute("srcObject")).toBe(null);
  el.srcObject = null;
  expect(el.getAttribute("src")).toBe(null);
  expect(el.srcObject).toBe(null);
  expect(() => {
    el.srcObject = { getVideoTracks: () => [] };
  }).toThrow(TypeError);
});

test("audio srcObject keeps an audio-only stream without playing it", () => {
  // A remote participant's track handed to a muted <audio> while Web Audio plays it.
  const rt = createRuntime();
  const el = rt.nodeOps.createElement("audio");
  const stream = { getVideoTracks: () => [], getAudioTracks: () => [{}] };
  el.srcObject = stream;
  expect(el.srcObject).toBe(stream);
  expect(el.getAttribute("src")).toBe(null);
  el.srcObject = null;
  expect(el.srcObject).toBe(null);
});
