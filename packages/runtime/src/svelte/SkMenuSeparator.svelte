<script lang="ts">
  import type { HTMLAttributes } from "svelte/elements";
  let { class: className, ...attrs }: HTMLAttributes<HTMLDivElement> = $props();
</script>

<div {...attrs} class={["sk-menu-separator", className]} role="separator"></div>
