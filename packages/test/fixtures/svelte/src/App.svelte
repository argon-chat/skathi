<script lang="ts">
  import { fade, slide } from "svelte/transition";
  import { add, list, rotate } from "./items.svelte";

  let { title }: { title: string } = $props();
  let count = $state(0);
  const doubled = $derived(count * 2);
  let open = $state(false);
</script>

<h1 id="title">{title}</h1>
<p id="count">count {count}, doubled {doubled}</p>
<div class="bar">
  <button id="inc" onclick={() => count++}>add one</button>
  <button id="toggle" onclick={() => (open = !open)}>toggle</button>
  <button id="add" onclick={() => add(`n${count}`)}>add item</button>
  <button id="rotate" onclick={rotate}>rotate</button>
</div>
{#if count > 1}
  <p id="many" transition:fade={{ duration: 120 }}>more than one</p>
{/if}
{#if open}
  <div id="panel" transition:slide={{ duration: 300 }}>panel</div>
{/if}
<ul id="list">
  {#each list.items as item (item.id)}
    <li class="item" data-id={item.id}>{item.label}</li>
  {/each}
</ul>
<p class="plain">unscoped</p>

<style>
  h1 {
    margin: 0;
    height: 30px;
    font-size: 20px;
    color: rgb(200, 0, 0);
  }
  p,
  ul {
    margin: 0;
    padding: 0;
  }
  #count {
    height: 20px;
  }
  #panel {
    padding: 6px 0;
    height: 40px;
  }
  .item {
    height: 20px;
  }
</style>
