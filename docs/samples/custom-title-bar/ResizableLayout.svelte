<script lang="ts">
  const folders = ["Inbox", "Drafts", "Sent", "Archive"];
  let current = $state("Inbox");
</script>

<div class="shell">
  <header class="titlebar">Mail</header>
  <div class="body">
    <nav class="sidebar">
      {#each folders as f (f)}
        <button class="folder" class:on={f === current} onclick={() => (current = f)}>{f}</button>
      {/each}
    </nav>
    <main class="list">{current}</main>
  </div>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .titlebar {
    display: flex;
    align-items: center;
    height: env(titlebar-area-height, 36px);
    padding-left: 12px;
    font-size: 12px;
    app-region: drag;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .sidebar {
    width: 200px;
    padding: 8px;
    border-right: 1px solid #80808040;
  }
  .folder {
    display: block;
    width: 100%;
    padding: 6px 10px;
    border-radius: 6px;
    text-align: left;
  }
  .folder.on {
    background: #80808033;
  }
  .list {
    flex: 1;
    padding: 16px;
  }
  /* #region reflow */
  @media (max-width: 640px) {
    .sidebar {
      display: none;
    }
  }
  /* #endregion reflow */
</style>
