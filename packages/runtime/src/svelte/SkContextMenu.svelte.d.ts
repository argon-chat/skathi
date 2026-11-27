import type { Component, Snippet } from "svelte";
import type { HTMLAttributes } from "svelte/elements";
import type { SkContextMenuHandle, SkMenuBackdrop } from "../context-menu";

type Props<T> = Omit<HTMLAttributes<HTMLDivElement>, "children" | "onclose"> & {
  backdrop?: SkMenuBackdrop;
  debugAim?: boolean;
  onclose?: () => void;
  children?: Snippet<[{ context: T | undefined }]>;
};
type Menu<T> = Component<Props<T>, SkContextMenuHandle<T>>;
declare const SkContextMenu: <T = unknown>(...args: Parameters<Menu<T>>) => ReturnType<Menu<T>>;
export default SkContextMenu;
