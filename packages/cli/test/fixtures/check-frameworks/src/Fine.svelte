<script lang="ts">
  import { fade } from "svelte/transition";

  let text = $state("");
  let size = $state("s");
  let open = $state(false);
</script>

<!-- {@html} and <input type="checkbox"> in a comment are not markup -->
<input bind:value={text} />
<select value={size} onchange={(e) => (size = e.currentTarget.value)}><option value="s">S</option></select>
{#if open}<p transition:fade>{text}</p>{/if}
<p>{"{@html x}"}</p>
