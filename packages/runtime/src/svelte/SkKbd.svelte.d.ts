import type { Component } from "svelte";
import type { HTMLAttributes } from "svelte/elements";
import type { SkKbdProps } from "../kbd";

declare const SkKbd: Component<HTMLAttributes<HTMLElement> & SkKbdProps>;
export default SkKbd;
