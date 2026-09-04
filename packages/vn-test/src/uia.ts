// The accessibility tree, read from OUTSIDE the process.
//
// Every other assertion in vn-test goes through the devtools wire, which is our
// own view of our own tree. This one does not: it asks Windows UI Automation
// what a screen reader would be told, through the same client API Narrator uses.
// If `crates/vn-tree/src/aria.rs` stopped lowering `role` and `aria-*`, every
// devtools assertion in the suite would still pass and this one would not.
//
// It is a PowerShell shell-out, and that is deliberate. UIA's client API is COM
// with a large surface; `System.Windows.Automation` is the managed binding
// Windows ships in the box, so the alternative is an FFI layer of our own for a
// helper that runs a handful of times per suite. The script is
// `src/uia.ps1` beside this file.
//
// Two properties of the walk matter:
//
// * **it never touches the window** — no activation, no focus, no synthetic
//   input. The user keeps their cursor.
// * **attaching a client is what turns accessibility ON.** gpui builds no
//   AccessKit tree until something asks for one (`A11y::is_active`), so the
//   first walk after launch can come back with only the window in it. That is
//   why {@link uiaTree} retries until the tree has content.

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
  /** `TogglePattern`: `"On" | "Off" | "Indeterminate"`. Absent when the node has no toggle state. */
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
  /** Give up after this many ms. Default 20 000. */
  timeout?: number;
  /**
   * Keep re-walking until the tree has at least this many nodes. Accessibility
   * is activated BY the first walk, so the tree is usually empty on it.
   * Default 2 (the window plus something in it).
   */
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
      // The script writes JSON on stdout; a per-node error (an element that
      // vanished between the walk and the read) goes to stderr and is not fatal.
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

/**
 * Walks the UI Automation tree of every top-level window `pid` owns.
 *
 * Retries until the tree has `minNodes` nodes, because the first walk is what
 * activates accessibility inside the target process — see the module comment.
 */
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
