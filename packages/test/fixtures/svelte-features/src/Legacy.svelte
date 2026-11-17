<script lang="ts">
  import { createEventDispatcher } from "svelte";

  export let start = 1;
  let count = start;
  $: doubled = count * 2;
  const dispatch = createEventDispatcher<{ bumped: number }>();

  function bump() {
    count += 1;
    dispatch("bumped", count);
  }
</script>

<button id="legacy-inc" on:click={bump}>legacy</button>
<span id="legacy">{count} {doubled}</span>
<slot name="extra" value={count}><span id="legacy-fallback">fallback</span></slot>
