import type { Component, Snippet } from "svelte";
import type { HTMLAttributes } from "svelte/elements";
import type { SkMenuBackdrop } from "../context-menu";

declare const SkSubmenu: Component<
  HTMLAttributes<HTMLDivElement> & { label: string; disabled?: boolean; backdrop?: SkMenuBackdrop; children?: Snippet }
>;
export default SkSubmenu;
