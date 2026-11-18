// Monitors, window placement and the per-monitor wallpaper, through user32 and PowerShell.
// Every coordinate is a physical virtual-screen pixel (per-monitor DPI aware v2).
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { dlopen, FFIType, JSCallback, ptr } from "bun:ffi";

const user32 = dlopen("user32.dll", {
  SetProcessDpiAwarenessContext: { args: [FFIType.i64], returns: FFIType.i32 },
  EnumDisplayMonitors: { args: [FFIType.u64, FFIType.ptr, FFIType.function, FFIType.i64], returns: FFIType.i32 },
  GetMonitorInfoW: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
  EnumWindows: { args: [FFIType.function, FFIType.i64], returns: FFIType.i32 },
  GetWindowThreadProcessId: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.u32 },
  IsWindowVisible: { args: [FFIType.u64], returns: FFIType.i32 },
  GetWindow: { args: [FFIType.u64, FFIType.u32], returns: FFIType.u64 },
  SetWindowPos: {
    args: [FFIType.u64, FFIType.u64, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.i32, FFIType.u32],
    returns: FFIType.i32,
  },
  GetWindowRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
  GetClientRect: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
  ClientToScreen: { args: [FFIType.u64, FFIType.ptr], returns: FFIType.i32 },
  PostMessageW: { args: [FFIType.u64, FFIType.u32, FFIType.u64, FFIType.i64], returns: FFIType.i32 },
});
const kernel32 = dlopen("kernel32.dll", {
  OpenProcess: { args: [FFIType.u32, FFIType.i32, FFIType.u32], returns: FFIType.u64 },
  QueryFullProcessImageNameW: { args: [FFIType.u64, FFIType.u32, FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
  CloseHandle: { args: [FFIType.u64], returns: FFIType.i32 },
});
const shcore = dlopen("shcore.dll", {
  GetDpiForMonitor: { args: [FFIType.u64, FFIType.i32, FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
});
const U = user32.symbols;
U.SetProcessDpiAwarenessContext(-4);

export type Rect = { left: number; top: number; width: number; height: number };
export type Monitor = Rect & { device: string; dpi: number; primary: boolean };

export const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export function monitors(): Monitor[] {
  const out: Monitor[] = [];
  const cb = new JSCallback(
    (hmon: bigint) => {
      const info = new Int32Array(26);
      info[0] = 104;
      if (U.GetMonitorInfoW(hmon, ptr(info))) {
        const dx = new Uint32Array(1);
        const dy = new Uint32Array(1);
        shcore.symbols.GetDpiForMonitor(hmon, 0, ptr(dx), ptr(dy));
        out.push({
          device: String.fromCharCode(...new Uint16Array(info.buffer, 40, 32)).replace(/\0.*$/, ""),
          left: info[1]!,
          top: info[2]!,
          width: info[3]! - info[1]!,
          height: info[4]! - info[2]!,
          primary: (info[9]! & 1) === 1,
          dpi: dx[0] || 96,
        });
      }
      return 1;
    },
    { args: [FFIType.u64, FFIType.u64, FFIType.ptr, FFIType.i64], returns: FFIType.i32 },
  );
  U.EnumDisplayMonitors(0n, null, cb, 0n);
  cb.close();
  return out;
}

/** The monitor to record on: `device` if given, else the only non-primary one. Never the primary. */
export function recordingMonitor(device?: string): Monitor {
  const all = monitors();
  const list = all.map(
    (m) => `${m.device}${m.primary ? " (primary)" : ""} ${m.width}x${m.height} at ${m.left},${m.top}`,
  );
  const pick = device
    ? all.find((m) => m.device.toUpperCase() === device.toUpperCase())
    : all.filter((m) => !m.primary).length === 1
      ? all.find((m) => !m.primary)
      : undefined;
  if (!pick) {
    throw new Error(
      device
        ? `no monitor ${device}; have: ${list.join("; ")}`
        : `need exactly one secondary monitor, or --display; have: ${list.join("; ")}`,
    );
  }
  if (pick.primary) throw new Error(`${pick.device} is the primary monitor; recording only goes on a secondary one`);
  return pick;
}

/** Each monitor's wallpaper path, keyed by device name; "" for a solid colour. */
export function wallpapers(): Map<string, string> {
  const r = spawnSync(
    "powershell",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", join(import.meta.dir, "wallpaper.ps1")],
    {
      encoding: "utf8",
    },
  );
  const out = new Map<string, string>();
  for (const line of (r.stdout ?? "").split(/\r?\n/)) {
    const [rect, path = ""] = line.split("\t");
    if (rect) out.set(rect, path.trim());
  }
  return out;
}

function imageOf(pid: number): string {
  const h = kernel32.symbols.OpenProcess(0x1000, 0, pid);
  if (!h) return "";
  const buf = new Uint16Array(1024);
  const size = new Uint32Array([1024]);
  const ok = kernel32.symbols.QueryFullProcessImageNameW(h, 0, ptr(buf), ptr(size));
  kernel32.symbols.CloseHandle(h);
  return ok ? String.fromCharCode(...buf.subarray(0, size[0])) : "";
}

function windowsOfImage(exeName: string): bigint[] {
  const found: bigint[] = [];
  const pidBuf = new Uint32Array(1);
  const images = new Map<number, string>();
  const cb = new JSCallback(
    (hwnd: bigint) => {
      U.GetWindowThreadProcessId(hwnd, ptr(pidBuf));
      const pid = pidBuf[0]!;
      if (!images.has(pid)) images.set(pid, imageOf(pid).toLowerCase());
      if (images.get(pid)!.endsWith(exeName.toLowerCase()) && U.GetWindow(hwnd, 4) === 0n) found.push(hwnd);
      return 1;
    },
    { args: [FFIType.u64, FFIType.i64], returns: FFIType.i32 },
  );
  U.EnumWindows(cb, 0n);
  cb.close();
  return found;
}

function visibleWindowsOf(targetPid: number): bigint[] {
  const found: bigint[] = [];
  const pidBuf = new Uint32Array(1);
  const cb = new JSCallback(
    (hwnd: bigint) => {
      U.GetWindowThreadProcessId(hwnd, ptr(pidBuf));
      if (pidBuf[0] === targetPid && U.IsWindowVisible(hwnd)) found.push(hwnd);
      return 1;
    },
    { args: [FFIType.u64, FFIType.i64], returns: FFIType.i32 },
  );
  U.EnumWindows(cb, 0n);
  cb.close();
  return found;
}

export function windowRect(hwnd: bigint): Rect {
  const r = new Int32Array(4);
  U.GetWindowRect(hwnd, ptr(r));
  return { left: r[0]!, top: r[1]!, width: r[2]! - r[0]!, height: r[3]! - r[1]! };
}

export function clientRect(hwnd: bigint): Rect {
  const r = new Int32Array(4);
  U.GetClientRect(hwnd, ptr(r));
  const p = new Int32Array(2);
  U.ClientToScreen(hwnd, ptr(p));
  return { left: p[0]!, top: p[1]!, width: r[2]!, height: r[3]! };
}

const SWP_NOSIZE = 0x1;
const SWP_NOZORDER = 0x4;
const SWP_NOACTIVATE = 0x10;
const WM_NCACTIVATE = 0x0086;

/** Moves the app's hidden windows onto `mon` as soon as they exist, so none is ever shown elsewhere. */
export function earlyPlacer(mon: Monitor, exeName: string): { stop(): void } {
  let stopped = false;
  void (async () => {
    while (!stopped) {
      for (const hwnd of windowsOfImage(exeName)) {
        const r = windowRect(hwnd);
        if (r.left >= mon.left && r.left < mon.left + mon.width) continue;
        U.SetWindowPos(hwnd, 0n, mon.left + 200, mon.top + 120, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
      }
      await sleep(25);
    }
  })();
  return { stop: () => (stopped = true) };
}

/** Puts the client area's centre on `mon`'s centre and raises the window without activating it. */
export function centre(hwnd: bigint, mon: Monitor): Rect {
  const outer = windowRect(hwnd);
  const client = clientRect(hwnd);
  const x = Math.round(mon.left + (mon.width - client.width) / 2 - (client.left - outer.left));
  const y = Math.round(mon.top + (mon.height - client.height) / 2 - (client.top - outer.top));
  U.SetWindowPos(hwnd, 0n, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
  return clientRect(hwnd);
}

/**
 * Keeps the process's windows drawn as active without taking the foreground: an inactive window
 * loses its acrylic and dims its caption, and the person at the machine keeps their focus.
 */
export function keepActive(pid: number): { stop(): void } {
  let keeping = true;
  void (async () => {
    while (keeping) {
      for (const w of visibleWindowsOf(pid)) U.PostMessageW(w, WM_NCACTIVATE, 1n, 0n);
      await sleep(120);
    }
  })();
  return { stop: () => (keeping = false) };
}
