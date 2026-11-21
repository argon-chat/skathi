<script lang="ts">
  import { appWindow } from "skathi:app";

  const tabs = $state(["Inbox", "Drafts", "Archive"]);
  let active = $state("Inbox");

  // #region overlay
  if (navigator.platform !== "MacIntel") appWindow.setTitleBarOverlay(true);
  // #endregion overlay
</script>

<div class="shell">
  <header class="titlebar">
    {#each tabs as tab (tab)}
      <button class="tab" class:active={tab === active} onclick={() => (active = tab)}>{tab}</button>
    {/each}
    <button class="tab" onclick={() => tabs.push(`Tab ${tabs.length + 1}`)}>+</button>
  </header>
  <main class="content">{active}</main>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .titlebar {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: env(titlebar-area-height, 40px);
    width: env(titlebar-area-width, 100%);
    padding-left: 8px;
    app-region: drag;
  }
  .tab {
    height: 32px;
    padding: 0 14px;
    border-radius: 8px 8px 0 0;
    font-size: 12px;
    app-region: no-drag;
  }
  .tab.active {
    background: #80808033;
  }
  .content {
    flex: 1;
    padding: 24px;
  }
</style>
