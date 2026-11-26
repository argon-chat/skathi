<script lang="ts">
  import { onDestroy, type Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { contextMenuRenderer, type SkMenuBackdrop } from "@skathi-native/runtime";
  const { createContextMenu, bindSubmenu } = contextMenuRenderer;
  let {
    label,
    disabled = false,
    backdrop,
    children,
    class: className,
    ...attrs
  }: HTMLAttributes<HTMLDivElement> & {
    label: string;
    disabled?: boolean;
    backdrop?: SkMenuBackdrop;
    children?: Snippet;
  } = $props();
  let visible = $state(false);
  const menu = createContextMenu(
    () => backdrop,
    (value) => {
      visible = value;
    },
  );
  onDestroy(menu.close);
  function trigger(el: HTMLElement) {
    return bindSubmenu(el, (keyboard) => {
      if (!disabled) menu.openSubmenu(el, keyboard);
    });
  }
  function panel(el: HTMLElement) {
    menu.attach(el);
    return () => menu.attach(null);
  }
</script>

<!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
<div
  {...attrs}
  class={["sk-menu-item", className]}
  role="menuitem"
  aria-haspopup="menu"
  aria-expanded={visible}
  aria-disabled={disabled}
  {@attach trigger}
  onclick={(event) => {
    if (!disabled) menu.openSubmenu(event.currentTarget, false);
  }}
>
  <span>{label}</span><span aria-hidden="true" class="sk-menu-arrow"></span>
</div>
{#if visible}
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    class="sk-context-menu"
    role="menu"
    aria-label={label}
    {@attach panel}
    onmousedown={(event) => event.preventDefault()}
  >
    {@render children?.()}
  </div>
{/if}
