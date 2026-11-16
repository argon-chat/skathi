<script lang="ts">
  const columns = Array.from({ length: 30 }, (_, i) => `Column ${i + 1}`);
  const rows = Array.from({ length: 40 }, (_, r) => columns.map((_, c) => `${r + 1}:${c + 1}`));

  // #region sync
  let header = $state<HTMLElement>();

  function onScroll(e: Event) {
    const left = (e.currentTarget as HTMLElement).scrollLeft;
    if (header) header.style.transform = `translateX(${-left}px)`;
  }
  // #endregion sync
</script>

<div class="table">
  <div class="header-clip">
    <div bind:this={header} class="header">
      {#each columns as name (name)}
        <div class="cell">{name}</div>
      {/each}
    </div>
  </div>
  <div class="body" onscroll={onScroll}>
    {#each rows as row, r (r)}
      <div class="row">
        {#each row as text, c (c)}
          <div class="cell">{text}</div>
        {/each}
      </div>
    {/each}
  </div>
</div>

<style>
  .header-clip {
    overflow: hidden;
  }
  .header,
  .row {
    display: flex;
  }
  .body {
    height: 360px;
    overflow: auto;
  }
  .cell {
    flex: none;
    width: 100px;
    height: 24px;
    padding: 0 6px;
    line-height: 24px;
  }
</style>
