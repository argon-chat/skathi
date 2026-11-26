<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { contextMenuRenderer } from "@skathi-native/runtime";
  const { selectMenuItem } = contextMenuRenderer;
  let {
    disabled = false,
    onselect,
    children,
    class: className,
    ...attrs
  }: Omit<HTMLAttributes<HTMLDivElement>, "onselect"> & {
    disabled?: boolean;
    onselect?: (event: MouseEvent) => void;
    children?: Snippet;
  } = $props();
</script>

<!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
<div
  {...attrs}
  class={["sk-menu-item", className]}
  role="menuitem"
  aria-disabled={disabled}
  onclick={(event) => selectMenuItem(event, disabled, () => onselect?.(event))}
>
  {@render children?.()}
</div>
