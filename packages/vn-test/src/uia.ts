// This one does not: it asks Windows UI Automation what a screen reader would be told, through the
// same client API Narrator uses.

import { spawn } from "node:child_process";
import { join } from "node:path";

/** One node of the UIA control view. */
export interface UiaNode {
  /** Depth below the top-level window, which is 0. */
  depth: number;
  /** The UIA control type, without its `ControlType.` prefix (`Button`, `CheckBox`, `Edit`, …). */
  type: string;
  /** The accessible name. */
  name: string;
  /** UIA `AutomationId` — AccessKit's author id. */
  autoId: string;
  enabled: boolean;
  offscreen: boolean;
  x: number;
  y: number;
  w: number;
  h: number;
  /** `TogglePattern`: `"On" | "Off" | "Indeterminate"`. */
  toggle?: "On" | "Off" | "Indeterminate";
  /** `SelectionItemPattern`. */
  selected?: boolean;
  /** `ExpandCollapsePattern`: `"Expanded" | "Collapsed" | "PartiallyExpanded" | "LeafNode"`. */
  expand?: string;
  /** `ValuePattern`. */
  value?: string;
  readonly?: boolean;
  rangeValue?: number;
  rangeMin?: number;
  rangeMax?: number;
  /** Heading level. */
  level?: number;
}

export interface UiaOptions {
  /** Give up after this many ms. */
  timeout?: number;
  /** Keep re-walking until the tree has at least this many nodes. */
  minNodes?: number;
}

const SCRIPT = join(import.meta.dir, "uia.ps1");

/** Windows only — UI Automation is a Windows API. */
export function uiaSupported(): boolean {
  return process.platform === "win32";
}

function runOnce(pid: number, timeout: number): Promise<UiaNode[]> {
  return new Promise((resolve, reject) => {
    const child = spawn(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", SCRIPT, "-ProcessId", String(pid)],
      { stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
    );
    let out = "";
    let err = "";
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`uiaTree(${pid}): the walk did not finish in ${timeout} ms`));
    }, timeout);
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (err += d));
    child.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      const start = out.indexOf("[");
      if (code !== 0 && start < 0) {
        reject(new Error(`uiaTree(${pid}): powershell exited ${code}\n${err}`));
        return;
      }
      if (start < 0) {
        resolve([]);
        return;
      }
      try {
        resolve(JSON.parse(out.slice(start)) as UiaNode[]);
      } catch (e) {
        reject(new Error(`uiaTree(${pid}): could not parse the walk\n${out.slice(0, 400)}`));
      }
    });
  });
}

/** Walks the UI Automation tree of every top-level window `pid` owns. */
export async function uiaTree(pid: number, opts: UiaOptions = {}): Promise<UiaNode[]> {
  if (!uiaSupported()) throw new Error("uiaTree is Windows-only");
  const timeout = opts.timeout ?? 20_000;
  const minNodes = opts.minNodes ?? 2;
  const deadline = Date.now() + timeout;
  let last: UiaNode[] = [];
  for (;;) {
    last = await runOnce(pid, Math.max(2_000, deadline - Date.now()));
    if (last.length >= minNodes) return last;
    if (Date.now() > deadline) return last;
    await new Promise((r) => setTimeout(r, 250));
  }
}

/** Every node whose accessible name is exactly `name`. */
export function uiaByName(nodes: UiaNode[], name: string): UiaNode[] {
  return nodes.filter((n) => n.name === name);
}

/** Every node of a given UIA control type. */
export function uiaByType(nodes: UiaNode[], type: string): UiaNode[] {
  return nodes.filter((n) => n.type === type);
}

/** A one-line-per-node rendering of the tree, for a failure message. */
export function uiaFormat(nodes: UiaNode[]): string {
  return nodes
    .map((n) => {
      const state = [
        n.toggle && `toggle=${n.toggle}`,
        n.selected !== undefined && `selected=${n.selected}`,
        n.expand && `expand=${n.expand}`,
        n.value !== undefined && `value=${JSON.stringify(n.value)}`,
        n.level !== undefined && `level=${n.level}`,
        n.enabled === false && "disabled",
        n.offscreen && "offscreen",
      ]
        .filter(Boolean)
        .join(" ");
      return `${"  ".repeat(n.depth)}${n.type} ${JSON.stringify(n.name)}${state ? ` [${state}]` : ""}`;
    })
    .join("\n");
}
