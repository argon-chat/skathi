<script lang="ts">
  const ROWS = 40;
  const COLS = 12;
  const cells = Array.from({ length: ROWS }, (_, r) => Array.from({ length: COLS }, (_, c) => `${r + 1}:${c + 1}`));

  // #region state
  const sel = $state({ row: 0, col: 0 });

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") sel.row = Math.min(ROWS - 1, sel.row + 1);
    else if (e.key === "ArrowUp") sel.row = Math.max(0, sel.row - 1);
    else if (e.key === "ArrowRight") sel.col = Math.min(COLS - 1, sel.col + 1);
    else if (e.key === "ArrowLeft") sel.col = Math.max(0, sel.col - 1);
  }
  // #endregion state
</script>

<div class="grid" role="grid" aria-label="Cells" tabindex="0" onkeydown={onKey}>
  {#each cells as row, r (r)}
    <div class="row" role="row">
      {#each row as text, c (c)}
        <div class="cell" role="gridcell">{text}</div>
      {/each}
    </div>
  {/each}
  <!-- #region overlay -->
  <div class="selection" style:transform="translate({sel.col * 80}px, {sel.row * 24}px)"></div>
  <!-- #endregion overlay -->
</div>

<style>
  .grid {
    position: relative;
  }
  .row {
    display: flex;
  }
  .cell {
    width: 80px;
    height: 24px;
    padding: 0 6px;
    line-height: 24px;
  }
  .selection {
    position: absolute;
    top: 0;
    left: 0;
    width: 80px;
    height: 24px;
    border: 2px solid #4c8dff;
    pointer-events: none;
  }
</style>
