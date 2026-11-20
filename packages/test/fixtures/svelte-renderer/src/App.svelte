<script lang="ts">
  import { getContext } from "svelte";
  import { SkWindow } from "@skathi-native/runtime/svelte";
  import DomField from "./DomField.svelte";

  let { heading }: { heading: string } = $props();
  const suffix = getContext<string>("suffix");
  let items = $state(["A", "B", "C"]);
  let clicks = $state(0);
  let show = $state(false);
  let childWindow = $state(false);
  let label = $derived(heading + suffix);
</script>

<h1>{label}</h1>
<div id="events" onclick={() => clicks++} role="presentation">
  <button id="add" onclick={() => (items = [...items.slice(1), items[0]!])}>Rotate</button>
</div>
<button id="toggle" onclick={() => (show = !show)}>Details</button>
<button id="window" onclick={() => (childWindow = !childWindow)}>Window</button>
<span id="clicks">{clicks}</span>
<DomField />
<button id="disabled" disabled={clicks === 0}>Bound attribute</button>
<svg viewBox="0 0 10 10" width="10" height="10"><defs><linearGradient id="gradient" /></defs></svg>
{#each items as item (item)}
  <span class="item" data-name={item}>{item}</span>
  <span class="separator">|</span>
{/each}
{#snippet detail(text: string)}<span class="detail">{text}</span>{/snippet}
{#if show}{@render detail("Details open")}{/if}
{#if childWindow}
  <SkWindow title="Renderer popout">{@render detail("In popout")}</SkWindow>
{/if}

<style>
  .item {
    color: rgb(30, 80, 130);
  }
</style>
