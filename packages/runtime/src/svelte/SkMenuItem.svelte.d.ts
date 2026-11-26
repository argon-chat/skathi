import type { Component, Snippet } from "svelte";
import type { HTMLAttributes } from "svelte/elements";

declare const SkMenuItem: Component<
  Omit<HTMLAttributes<HTMLDivElement>, "onselect"> & {
    disabled?: boolean;
    onselect?: (event: MouseEvent) => void;
    children?: Snippet;
  }
>;
export default SkMenuItem;
