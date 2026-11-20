import { afterAll, expect, test } from "bun:test";
import { dlopen, FFIType, ptr } from "bun:ffi";
import { launch, MACOS, WINDOWS } from "../src/index";

const wayland = process.platform === "linux" && !!process.env.WAYLAND_DISPLAY;
const app = await launch({
  app: "packages/test/fixtures/call-timer/src/main.ts",
  window: [320, 200],
  label: "window-controls",
});
const user32 = WINDOWS
  ? dlopen("user32.dll", {
      GetWindowRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
      GetClientRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
      GetLayeredWindowAttributes: { args: [FFIType.u64, FFIType.ptr, FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
      SendMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.ptr], returns: FFIType.i64 },
      GetWindowLongPtrW: { args: [FFIType.u64, FFIType.i32], returns: FFIType.i64 },
      GetWindow: { args: [FFIType.u64, FFIType.u32], returns: FFIType.u64 },
    })
  : null;
afterAll(async () => {
  user32?.close();
  await app.close();
});
const run = (body: string) => app.eval(`(() => { const w = __SK_WINDOW__.appWindow; ${body} })()`);
async function until(expression: string) {
  const end = Date.now() + 8000;
  while (Date.now() < end) {
    if ((await run(`__SK_WINDOW__.refresh(); return String(!!(${expression}));`)) === "true") return;
    await Bun.sleep(35);
  }
  throw new Error(`Timed out waiting for ${expression}. Window state: ${await run("return __sk_window_state()")}`);
}

test("live content limits resize an existing window and constrain later outer bounds", async () => {
  try {
    await run("w.setMinSize({width: 410, height: 230})");
    await until("w.chrome.client.width >= 409 && w.chrome.client.height >= 229");
    await run("w.setBounds({width: 190, height: 160})");
    await until("Math.abs(w.chrome.client.width - 410) <= 1 && Math.abs(w.chrome.client.height - 230) <= 1");
    await run("w.setMinSize(null); w.setMaxSize({width: 290, height: 185})");
    await until("w.chrome.client.width <= 291 && w.chrome.client.height <= 186");
    await run("w.setBounds({width: 600, height: 400})");
    await until("Math.abs(w.chrome.client.width - 290) <= 1 && Math.abs(w.chrome.client.height - 185) <= 1");
  } finally {
    await run("w.setMinSize(null); w.setMaxSize(null)");
  }
  await run("w.setBounds({width: 420, height: 290})");
  await until("Math.abs(w.getBounds().width - 420) <= 1 && Math.abs(w.getBounds().height - 290) <= 1");
}, 30000);

test("aspect ratio follows the content area for width-only and height-only resizes", async () => {
  try {
    await run("w.setAspectRatio(16/9); w.setContentBounds({width: 384})");
    await until("Math.abs(w.chrome.client.width - 384) <= 1 && Math.abs(w.chrome.client.height - 216) <= 1");
    await run("w.setContentBounds({height: 180})");
    await until("Math.abs(w.chrome.client.width - 320) <= 1 && Math.abs(w.chrome.client.height - 180) <= 1");
    if (user32) {
      const hwnd = await app.hwnd();
      const frame = new Int32Array(4),
        client = new Int32Array(4);
      expect(user32.symbols.GetWindowRect(hwnd, ptr(frame))).toBe(1);
      expect(user32.symbols.GetClientRect(hwnd, ptr(client))).toBe(1);
      const borderW = frame[2]! - frame[0]! - client[2]!,
        borderH = frame[3]! - frame[1]! - client[3]!;
      const left = frame[0]!,
        top = frame[1]!;
      frame[2] = frame[2]! + 80;
      user32.symbols.SendMessageW(hwnd, 0x214, 2n, ptr(frame)); // WM_SIZING, right edge
      expect(frame[0]).toBe(left);
      expect(frame[1]).toBe(top);
      expect(Math.abs(((frame[2]! - left - borderW) * 9) / 16 - (frame[3]! - top - borderH))).toBeLessThanOrEqual(1);
    }
  } finally {
    await run("w.setAspectRatio(null)");
  }
  await run("w.setContentBounds({width: 340, height: 250})");
  await until("Math.abs(w.chrome.client.height - 250) <= 1 && Math.abs(w.chrome.client.width - 340) <= 1");
}, 30000);

test("animated bounds relayout intermediate frames and land at the requested rectangle", async () => {
  await run(`w.setBounds({width:320,height:200}); globalThis.__glideWidths=[];
    w.addEventListener('resize',()=>__glideWidths.push([w.getBounds().width, w.chrome.client.width]));`);
  await until("Math.abs(w.getBounds().width - 320) <= 1");
  await run("w.setBounds({width:520,height:330},{animate:{duration:450,easing:'linear'}})");
  await until("Math.abs(w.getBounds().width-520)<=1 && Math.abs(w.getBounds().height-330)<=1");
  const samples: [number, number][] = JSON.parse(await run("return JSON.stringify(__glideWidths)"));
  expect(samples.some(([outer, client]) => outer > 325 && outer < 515 && client > 300 && client <= outer + 1)).toBe(
    true,
  );
  await run("w.setBounds({width:350},{animate:true}); w.setBounds({width:460})");
  await Bun.sleep(300);
  await until("Math.abs(w.getBounds().width - 460) <= 1");
}, 30000);

test("faded visibility, retained opacity, and interrupted transitions", async () => {
  await run("w.opacity = 0.6");
  if (wayland) {
    expect(await run("return String(w.opacity)")).toBe("1");
    return;
  }
  await until("Math.abs(w.opacity - 0.6) < 0.01");
  if (user32) {
    const alpha = new Uint8Array(1);
    expect(user32.symbols.GetLayeredWindowAttributes(await app.hwnd(), null, ptr(alpha), null)).toBe(1);
    expect(alpha[0]).toBe(153);
  }
  await run("w.hide({fade:400})");
  await until("w.opacity>0 && w.opacity<0.59");
  await run("w.show({fade:120})");
  await until("w.isVisible() && Math.abs(w.opacity-0.6)<0.01");
  await run("w.hide({fade:100})");
  await until("!w.isVisible()");
  await run("w.show({fade:130})");
  await until("w.isVisible() && Math.abs(w.opacity-0.6)<0.01");
  await run("w.opacity = 1");
}, 30000);

test("the mini player can request topmost and macOS Spaces independently", async () => {
  await run("w.alwaysOnTop=true; w.visibleOnAllWorkspaces=true");
  await until(wayland ? "!w.alwaysOnTop" : "w.alwaysOnTop");
  await until(WINDOWS || wayland ? "!w.visibleOnAllWorkspaces" : "w.visibleOnAllWorkspaces");
  await run("w.alwaysOnTop=false; w.visibleOnAllWorkspaces=false");
  await until("!w.alwaysOnTop && !w.visibleOnAllWorkspaces");
}, 30000);

test.if(MACOS)(
  "both pinning flags move the same content to a real nonactivating panel",
  async () => {
    type Native = {
      window_class: string;
      window_number: number;
      workspace_panel: boolean;
      content_window_number: number;
      nonactivating_panel: boolean;
      can_become_main: boolean;
      can_become_key: boolean;
      collection_behavior: number;
      level: number;
    };
    const native = async () => {
      const reply = await app.send("windows", { native: true });
      return (reply.windows as { kind: string; native: Native }[]).find((w) => w.kind === "main")!.native;
    };
    const waitForWindow = async (number: number) => {
      const deadline = Date.now() + 8000;
      let current = await native();
      while (current.window_number !== number || current.content_window_number !== number) {
        if (Date.now() >= deadline)
          throw new Error(`Expected window and content on ${number}, got ${JSON.stringify(current)}`);
        await Bun.sleep(35);
        current = await native();
      }
      return current;
    };
    const checkPanel = (panel: Native) => {
      expect(panel.window_class).toBe("GPUIWorkspacePanel");
      expect(panel.workspace_panel && panel.nonactivating_panel && panel.can_become_key && !panel.can_become_main).toBe(
        true,
      );
      expect(panel.content_window_number).toBe(panel.window_number);
      expect(panel.collection_behavior & 257).toBe(257);
      expect(panel.level).toBe(3);
    };
    const original = await native();
    await run(
      "globalThis.__miniInput = document.createElement('input'); __miniInput.id = 'mini-input'; __miniInput.style.cssText='position:fixed;left:0;top:0;width:120px;height:30px'; document.body.appendChild(__miniInput); __miniInput.value = 'before'",
    );
    try {
      for (const order of [
        ["visibleOnAllWorkspaces", "alwaysOnTop"],
        ["alwaysOnTop", "visibleOnAllWorkspaces"],
      ]) {
        await run(`w.${order[0]} = true`);
        await until(`w.${order[0]}`);
        expect((await native()).window_number).toBe(original.window_number);
        await run(`w.setBounds({width: 386}); w.${order[1]} = true`);
        await until("w.visibleOnAllWorkspaces && w.alwaysOnTop");
        const panel = await native();
        checkPanel(panel);
        expect(panel.window_number).not.toBe(original.window_number);
        await until("Math.abs(w.getBounds().width - 386) <= 1");
        await app.locator("#mini-input").type("x");
        expect(String(await run("return __miniInput.value")).includes("x")).toBe(true);
        if (order[0] === "visibleOnAllWorkspaces") {
          await run("w.setFullscreen(true, {transition: 'fade'})");
          await until("w.isFullscreen()");
          const primary = await waitForWindow(original.window_number);
          expect(primary.workspace_panel).toBe(false);
          await run("w.setFullscreen(false, {transition: 'fade'})");
          await until("!w.isFullscreen()");
          checkPanel(await waitForWindow(panel.window_number));
        }
        await run(`w.${order[0]} = false`);
        await until(`!w.${order[0]} && w.${order[1]}`);
        const restored = await waitForWindow(original.window_number);
        expect(restored.workspace_panel).toBe(false);
        expect(restored.level).toBe(order[1] === "alwaysOnTop" ? 3 : 0);
        await run(`w.${order[1]} = false`);
        await until("!w.visibleOnAllWorkspaces && !w.alwaysOnTop");
        expect((await native()).window_number).toBe(original.window_number);
      }
    } finally {
      await run("w.alwaysOnTop=false; w.visibleOnAllWorkspaces=false; __miniInput.remove()");
    }
  },
  30000,
);

test.if(WINDOWS)(
  "topmost is above a normal borderless fullscreen window on the same desktop",
  async () => {
    if (!user32) return;
    const other = await launch({
      app: "packages/test/fixtures/call-timer/src/main.ts",
      label: "fullscreen-under-mini",
    });
    try {
      await other.eval("__SK_WINDOW__.appWindow.setFullscreen(true); true");
      const deadline = Date.now() + 8000;
      while (!(await other.eval("__SK_WINDOW__.appWindow.isFullscreen()"))) {
        if (Date.now() >= deadline) throw new Error("Borderless window did not enter fullscreen");
        await Bun.sleep(35);
      }
      await run("w.alwaysOnTop=true");
      await until("w.alwaysOnTop");
      const top = await app.hwnd(),
        lower = await other.hwnd();
      expect(Number(user32.symbols.GetWindowLongPtrW(top, -20)) & 8).toBe(8);
      let cursor = top;
      const visited = new Set<bigint>();
      while (cursor && cursor !== lower && !visited.has(cursor)) {
        visited.add(cursor);
        cursor = BigInt(user32.symbols.GetWindow(cursor, 2));
      }
      expect(cursor).toBe(lower);
    } finally {
      await run("w.alwaysOnTop=false");
      await other.close();
    }
  },
  30000,
);

test("fullscreen events bracket the transition and preserve the traffic light override", async () => {
  await run(`w.setTitleBarOverlay({height:46}); w.setTrafficLightPosition({x:22,y:10});
    globalThis.__fsEdges=[];
    w.addEventListener('fullscreenwillchange', e=>__fsEdges.push('will:'+e.detail.fullscreen));
    w.addEventListener('fullscreenchange', ()=>__fsEdges.push('did:'+w.isFullscreen()));`);
  if (MACOS) await until("w.chrome.trafficLightPosition?.x === 22 && w.chrome.trafficLightPosition?.y === 10");
  await run("w.setWindowButtonsVisible(false); w.setFullscreen(true, {transition:'fade'})");
  await until("__fsEdges.includes('did:true')");
  await until("w.getWindowButtonsRect() === null");
  await run("w.setFullscreen(false, {transition:'fade'})");
  await until("__fsEdges.includes('did:false')");
  expect(JSON.parse(await run("return JSON.stringify(__fsEdges)"))).toEqual([
    "will:true",
    "did:true",
    "will:false",
    "did:false",
  ]);
  if (MACOS) await until("w.chrome.trafficLightPosition?.x === 22 && w.chrome.trafficLightPosition?.y === 10");
  await until("w.getWindowButtonsRect() === null");
  await run("w.setWindowButtonsVisible(true); w.setTitleBarOverlay({height:50})");
  if (MACOS || WINDOWS) await until("w.getWindowButtonsRect()?.width > 0 && w.getWindowButtonsRect()?.height > 0");
  await run("w.setTitleBarOverlay({height:46})");
  await run("w.setTrafficLightPosition(null)");
  if (MACOS) await until("w.chrome.trafficLightPosition?.x === 12 && w.chrome.trafficLightPosition?.y === 16");
}, 30000);
