import { expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

test("desktop source list crosses the asynchronous bridge as an array", async () => {
  const rt = createRuntime();
  const sources = [{ id: "screen:123", name: "Display", thumbnailDataUrl: "data:image/png;base64,x", appIconDataUrl: null, displayId: "123" }];
  const pending = rt.appCapture.getSources(["screen"], { withIcons: false });
  expect(rt.host.desktop.calls.at(-1)).toEqual({ op: "screenSources", args: [["screen"], { withIcons: false }] });
  rt.host.desktop.settle(1, true, sources);
  expect(await pending).toEqual(sources);
});

test("invalid source types and missing native capture reject explicitly", async () => {
  const rt = createRuntime();
  await expect(rt.appCapture.getSources(["tab"])).rejects.toThrow(TypeError);
  expect(rt.host.desktop.calls).toEqual([]);
  await expect(createRuntime({ desktop: false }).appCapture.getSources()).rejects.toMatchObject({ name: "NotSupportedError" });
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
  expect(() => { el.srcObject = { getVideoTracks: () => [] }; }).toThrow(TypeError);
});
