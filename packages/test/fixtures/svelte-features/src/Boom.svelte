<script lang="ts">
  let { fail }: { fail: boolean } = $props();
  const value = $derived.by(() => {
    if (fail) throw new Error("render-boom");
    return "fine";
  });
</script>

<span id="boom">{value}</span>
