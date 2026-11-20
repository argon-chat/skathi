<svelte:options customRenderer={null} />

<script lang="ts">
  let value = $state("mixed");
</script>

<input class="dom-field" bind:value />
<span class="dom-value">{value}</span>

<style>
  .dom-field {
    width: 160px;
  }
</style>
