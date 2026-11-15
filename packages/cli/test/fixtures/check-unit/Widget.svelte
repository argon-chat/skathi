<script lang="ts">
  let count = 0;
  const twice = eval("2") as number;
</script>

<svelte:head>
  <script>eval("head");</script>
</svelte:head>
<!-- <script>eval("comment")</script> -->

<button onclick={() => count++}>{count * twice}</button>
