<script lang="ts">
  import { appWindow } from "@skathi-native/runtime/svelte";

  // #region state
  const state = $derived.by(() => {
    if (appWindow.isFullscreen()) return "fullscreen";
    if (appWindow.isMinimized()) return "minimized";
    return appWindow.isMaximized() ? "maximized" : "normal";
  });
  // #endregion state
</script>

<div class="toolbar">
  <!-- #region buttons -->
  <button class="btn" onclick={() => appWindow.minimize()}>Minimize</button>
  <button class="btn" onclick={() => appWindow.maximize()}>Maximize</button>
  <button class="btn" onclick={() => appWindow.unmaximize()}>Restore</button>
  <button class="btn" onclick={() => appWindow.toggleMaximize()}>Toggle</button>
  <!-- #endregion buttons -->
  <span class="state">State: {state}</span>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px;
  }
  .btn {
    padding: 6px 12px;
    border-radius: 6px;
    background: #ffffff14;
    color: #f3f3f3;
  }
  .state {
    margin-left: auto;
    font-size: 12px;
    color: #ffffffb0;
  }
</style>
