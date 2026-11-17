<script lang="ts">
  import { hydrate } from "svelte";
  import { flip } from "svelte/animate";

  let done = $state(false);
  let size = $state("s");
  let items = $state([1, 2]);
  export const later = hydrate;
</script>

<div>{@html "<b>hi</b>"}</div>
<input type="checkbox" bind:checked={done} />
<select bind:value={size}><option value="s">S</option></select>
<ul>
  {#each items as item (item)}
    <li animate:flip>{item}</li>
  {/each}
</ul>

<style lang="scss">
  ul {
    margin: 0;
  }
</style>
