<script lang="ts">
  import { appWindow } from "skathi:app";
</script>

<header class="titlebar">
  <span class="title">{appWindow.title}</span>
  <div class="controls">
    <div class="ctl min" title="Minimize">–</div>
    <div class="ctl max" title={appWindow.isMaximized() ? "Restore" : "Maximize"}>
      {appWindow.isMaximized() ? "❐" : "□"}
    </div>
    <div class="ctl close" title="Close">✕</div>
  </div>
</header>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    height: env(titlebar-area-height, 36px);
    app-region: drag;
    user-select: none;
  }
  .title {
    flex: 1;
    padding-left: 12px;
    font-size: 12px;
  }
  .controls {
    display: flex;
    height: 100%;
  }
  .ctl {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: 100%;
    font-size: 12px;
  }
  .ctl:hover {
    background: #80808026;
  }
  /* #region controls */
  .min {
    --window-control: minimize;
  }
  .max {
    --window-control: maximize;
  }
  .close {
    --window-control: close;
  }
  /* #endregion controls */
  .close:hover {
    background: #c42b1c;
    color: #ffffff;
  }
</style>
