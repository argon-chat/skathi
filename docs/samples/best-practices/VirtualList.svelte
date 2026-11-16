<script lang="ts">
  const ROW = 28;
  const HEIGHT = 420;
  const OVERSCAN = 4;
  const rows = Array.from({ length: 10_000 }, (_, i) => ({ id: i, text: `Row ${i + 1}` }));

  // #region slice
  let first = $state(0);
  const visible = $derived(rows.slice(first, first + Math.ceil(HEIGHT / ROW) + 2 * OVERSCAN));

  function onScroll(e: Event) {
    const top = (e.currentTarget as HTMLElement).scrollTop;
    first = Math.max(0, Math.floor(top / ROW) - OVERSCAN);
  }
  // #endregion slice
</script>

<!-- #region template -->
<div class="viewport" onscroll={onScroll}>
  <div class="spacer" style:height="{rows.length * ROW}px">
    {#each visible as row (row.id)}
      <div class="row" style:top="{row.id * ROW}px">{row.text}</div>
    {/each}
  </div>
</div>

<!-- #endregion template -->

<style>
  .viewport {
    height: 420px;
    overflow-y: auto;
  }
  .spacer {
    position: relative;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 28px;
    padding: 0 12px;
    line-height: 28px;
  }
</style>
