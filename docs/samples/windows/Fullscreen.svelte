<script lang="ts">
  import { appWindow } from "skathi:app";

  function toggleFullscreen() {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void document.documentElement.requestFullscreen();
  }
</script>

<div class="player">
  <button class="btn" onclick={toggleFullscreen}>
    {appWindow.isFullscreen() ? "Leave fullscreen" : "Fullscreen"}
  </button>
  <p class="hint">Press Escape to leave.</p>
</div>

<style>
  .player {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 100%;
  }
  .btn {
    padding: 8px 16px;
    border-radius: 6px;
    background: #ffffff14;
    color: #f3f3f3;
  }
  .hint {
    font-size: 12px;
    color: #ffffff90;
  }
</style>
