<script lang="ts">
  // sk.config.json hides the OS title bar, so the app paints its own: `-webkit-app-region: drag` drags the
  // window, and `--window-control` gives each button its OS behaviour (snap layouts on maximize).
  import { appWindow } from "@skathi-native/runtime";
  import { onMount } from "svelte";
  import { look, SCENES, setScene } from "../lib/fx.svelte";
  import { productName } from "../lib/product";

  let hostControls = $state(appWindow.chrome.hostControls);
  let maximized = $state(appWindow.isMaximized());

  onMount(() => {
    const read = () => {
      hostControls = appWindow.chrome.hostControls;
      maximized = appWindow.isMaximized();
    };
    navigator.windowControlsOverlay.addEventListener("geometrychange", read);
    appWindow.addEventListener("maximize", read);
    appWindow.addEventListener("unmaximize", read);
    return () => {
      navigator.windowControlsOverlay.removeEventListener("geometrychange", read);
      appWindow.removeEventListener("maximize", read);
      appWindow.removeEventListener("unmaximize", read);
    };
  });
</script>

<header class="titlebar">
  <div class="brand">
    <img class="logo" src="assets/wolf.svg" alt="" />
    <span class="name">{productName}</span>
  </div>

  <div class="scenes" role="radiogroup" aria-label="Backdrop">
    {#each SCENES as scene (scene.id)}
      <button
        class="scene"
        class:on={look.scene === scene.id}
        role="radio"
        aria-checked={look.scene === scene.id}
        onclick={() => setScene(scene.id)}
      >
        {scene.label}
      </button>
    {/each}
  </div>

  <div class="end">
    {#if !hostControls}
      <div class="controls">
        <div class="control minimize" title="Minimize">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" /></svg>
        </div>
        <div class="control maximize" title={maximized ? "Restore" : "Maximize"}>
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            {#if maximized}
              <path d="M2.5 2.5V.5h7v7h-2 M.5 2.5h7v7h-7z" stroke="currentColor" />
            {:else}
              <path d="M.5 .5h9v9h-9z" stroke="currentColor" />
            {/if}
          </svg>
        </div>
        <div class="control close" title="Close">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" stroke="currentColor" /></svg>
        </div>
      </div>
    {/if}
  </div>
</header>

<style>
  .titlebar {
    height: env(titlebar-area-height, 44px);
    padding-left: env(titlebar-area-x, 0px);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    -webkit-app-region: drag;
    user-select: none;
  }
  @media (display-mode: fullscreen) {
    .titlebar {
      display: none;
    }
  }
  .brand {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 9px;
    padding-left: 16px;
  }
  .logo {
    width: 26px;
    height: 26px;
  }
  .name {
    font-size: 13px;
    font-weight: 700;
    color: #edeaf3;
  }
  .scenes {
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: 14px;
    background: #ffffff0f;
    border: 1px solid #ffffff14;
    -webkit-app-region: no-drag;
  }
  .scene {
    height: 24px;
    padding: 0 12px;
    border: none;
    border-radius: 11px;
    background: transparent;
    color: #a9a2bb;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition:
      background-color 160ms ease,
      color 160ms ease;
  }
  .scene:hover {
    color: #edeaf3;
  }
  .scene.on {
    background: #ffffff24;
    color: #ffffff;
  }
  .end {
    flex: 1;
    height: 100%;
    display: flex;
    justify-content: flex-end;
  }
  .controls {
    height: 100%;
    display: flex;
    -webkit-app-region: no-drag;
  }
  .control {
    width: 46px;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #b3acc4;
    transition:
      background-color 120ms ease,
      color 120ms ease;
  }
  .control:hover {
    background: #ffffff1a;
    color: #ffffff;
  }
  .minimize {
    --window-control: minimize;
  }
  .maximize {
    --window-control: maximize;
  }
  .close {
    --window-control: close;
  }
  .close:hover {
    background: #c42b1c;
    color: #ffffff;
  }
</style>
