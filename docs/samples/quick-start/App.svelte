<script lang="ts">
  import { appWindow } from "skathi:app";

  // #region state
  let count = $state(0);
  const label = $derived(appWindow.isMaximized() ? "Restore" : "Maximize");
  // #endregion state
</script>

<main class="page">
  <h1>{appWindow.title}</h1>
  <button onclick={() => count++}>Clicked {count} times</button>
  <button onclick={() => appWindow.toggleMaximize()}>{label}</button>
</main>

<style>
  .page {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 32px;
    color: #f4f4f5;
  }
</style>
