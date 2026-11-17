<script lang="ts">
  import { getContext } from "svelte";
  import type { Snippet } from "svelte";

  let { value = $bindable(""), row }: { value?: string; row: Snippet<[string]> } = $props();
  const fromParent = getContext<string>("ctx");
</script>

<input id="bindable" bind:value />
<span id="ctx">{fromParent}</span>
{@render row("snip")}
