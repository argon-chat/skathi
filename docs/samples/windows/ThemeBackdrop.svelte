<script lang="ts">
  import { appWindow } from "skathi:app";

  const THEMES = ["system", "light", "dark"];
  const BACKDROPS = ["mica", "acrylic", "none"];

  // #region persist
  appWindow.theme = localStorage.getItem("theme") ?? "system";

  function setTheme(theme: string) {
    appWindow.theme = theme;
    localStorage.setItem("theme", theme);
  }
  // #endregion persist
</script>

<div class="panel">
  <div class="row">
    {#each THEMES as t (t)}
      <button class="chip" class:on={appWindow.theme === t} onclick={() => setTheme(t)}>{t}</button>
    {/each}
  </div>
  <div class="row">
    {#each BACKDROPS as b (b)}
      <button class="chip" class:on={appWindow.backdrop === b} onclick={() => (appWindow.backdrop = b)}>{b}</button>
    {/each}
  </div>
  <p>Drawn: {appWindow.effectiveBackdrop}</p>
</div>

<style>
  .panel {
    --fg: #1b1b1b;
    --chip: #0000000f;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    color: var(--fg);
  }
  @media (prefers-color-scheme: dark) {
    .panel {
      --fg: #f3f3f3;
      --chip: #ffffff14;
    }
  }
  .row {
    display: flex;
    gap: 6px;
  }
  .chip {
    padding: 6px 12px;
    border-radius: 6px;
    background: var(--chip);
    color: var(--fg);
  }
  .chip.on {
    background: #0067c0;
    color: #ffffff;
  }
</style>
