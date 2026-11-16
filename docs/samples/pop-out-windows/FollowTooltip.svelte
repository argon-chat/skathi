<script lang="ts">
  import { appWindow, SkWindow, type SkWindowExports, type SkWindowRect } from "@skathi-native/runtime/svelte";

  const isWindows = navigator.platform === "Win32";
  const tracks = Array.from({ length: 40 }, (_, i) => `Track ${i + 1}`);
  let selected = $state<number | null>(null);
  let list = $state<HTMLElement>();
  let overlay = $state<SkWindowExports>();
  let tip = $state<SkWindowRect | null>(null);
  const rows: HTMLElement[] = [];
  const label = $derived(selected === null ? "" : `${tracks[selected]}, 3:41`);

  function place() {
    const row = selected === null ? undefined : rows[selected];
    const r = row && overlay ? overlay.mapFromMain(row) : null;
    const box = overlay ? overlay.mapFromMain(list) : null;
    if (r && box && r.y + r.height > box.y && r.y < box.y + box.height) tip = r;
    else tip = null;
  }

  let frame = 0;
  function schedule() {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(place);
  }

  function select(i: number) {
    selected = selected === i ? null : i;
    schedule();
  }

  $effect(() => {
    appWindow.addEventListener("move", schedule);
    return () => {
      appWindow.removeEventListener("move", schedule);
      cancelAnimationFrame(frame);
    };
  });
</script>

<svelte:window onresize={schedule} />

<div bind:this={list} class="list" onscroll={schedule}>
  {#each tracks as t, i (t)}
    <button bind:this={rows[i]} class="row" class:on={selected === i} onclick={() => select(i)}>{t}</button>
  {/each}
</div>
{#if isWindows}
  <SkWindow
    bind:this={overlay}
    title="Track details"
    cover="monitor"
    chrome="hidden"
    backdrop="transparent"
    alwaysOnTop
    ignoreMouseEvents
    focusable={false}
    visible={tip !== null}
  >
    <div class="layer">
      {#if tip}
        <div class="tip" style:left="{tip.x + tip.width + 12}px" style:top="{tip.y}px">{label}</div>
      {/if}
    </div>
  </SkWindow>
{/if}

<style>
  .list {
    height: 100%;
    overflow-y: auto;
    padding: 8px;
  }
  .row {
    display: block;
    width: 100%;
    padding: 8px 12px;
    border-radius: 6px;
    text-align: left;
  }
  .row.on {
    background: #0090ff40;
  }
  .layer {
    position: relative;
  }
  .tip {
    position: absolute;
    padding: 8px 12px;
    border-radius: 8px;
    background: #1b1b1bf0;
    color: #ffffff;
    font-size: 12px;
  }
</style>
