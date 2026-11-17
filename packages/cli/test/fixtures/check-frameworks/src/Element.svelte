<svelte:options customElement="my-element" />

<script lang="ts">
  const host = $host();
</script>

<p>{host.tagName}</p>
