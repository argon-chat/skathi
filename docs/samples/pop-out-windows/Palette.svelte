<script lang="ts">
  import { SkWindow, type SkWindowBounds } from "@skathi-native/runtime/svelte";
  import { appWindow } from "skathi:app";

  const colors = ["#e5484d", "#f76b15", "#ffc53d", "#30a46c", "#0090ff", "#8e4ec6"];
  let ink = $state("#0090ff");
  let open = $state(false);
  let bounds = $state<SkWindowBounds>({ width: 200, height: 260 });

  function showPalette() {
    if (navigator.platform === "Win32") {
      const main = appWindow.getBounds();
      bounds = { x: main.x + main.width + 8, y: main.y, width: 200, height: 260 };
    }
    open = true;
  }
</script>

<main class="editor">
  <div class="canvas" style:background={ink}></div>
  {#if !open}
    <button onclick={showPalette}>Show palette</button>
  {:else}
    <SkWindow title="Palette" {bounds} taskbar={false} minWidth={160} onclose={() => (open = false)}>
      <div class="swatches">
        {#each colors as c (c)}
          <button class="swatch" class:on={c === ink} style:background={c} title={c} onclick={() => (ink = c)}></button>
        {/each}
      </div>
    </SkWindow>
  {/if}
</main>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 24px;
  }
  .canvas {
    height: 240px;
    border-radius: 12px;
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(3, 48px);
    gap: 10px;
    padding: 16px;
  }
  .swatch {
    width: 48px;
    height: 48px;
    border-radius: 24px;
  }
  .swatch.on {
    border: 3px solid #ffffff;
  }
</style>
