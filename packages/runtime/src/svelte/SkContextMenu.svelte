<script lang="ts" generics="T = unknown">
  import { onDestroy, type Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { contextMenuRenderer, type SkMenuAnchor, type SkMenuBackdrop } from "@skathi-native/runtime";
  const { createContextMenu } = contextMenuRenderer;

  let {
    backdrop,
    debugAim = false,
    onclose,
    children,
    class: className,
    ...attrs
  }: Omit<HTMLAttributes<HTMLDivElement>, "children" | "onclose"> & {
    backdrop?: SkMenuBackdrop;
    debugAim?: boolean;
    onclose?: () => void;
    children?: Snippet<[{ context: T | undefined }]>;
  } = $props();
  let visible = $state(false);
  let context = $state.raw<T | undefined>();
  const menu = createContextMenu<T>(
    () => backdrop,
    (value, data) => {
      visible = value;
      context = data;
      if (!value) onclose?.();
    },
    () => debugAim,
  );

  export function open(anchor: SkMenuAnchor, context?: T) {
    menu.open(anchor, context);
  }
  export function close() {
    menu.close();
  }
  onDestroy(close);
  function attach(el: HTMLElement) {
    menu.attach(el);
    return () => menu.attach(null);
  }
</script>

{#if visible}
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    {...attrs}
    class={["sk-context-menu", className]}
    role="menu"
    {@attach attach}
    onmousedown={(event) => event.preventDefault()}
  >
    {@render children?.({ context })}
  </div>
{/if}
