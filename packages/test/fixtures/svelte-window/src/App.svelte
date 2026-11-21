<script lang="ts">
  import { onDestroy } from "svelte";
  import { mount, SkWindow, type SkWindowExports, type SkWindowMetrics } from "@skathi-native/runtime/svelte";
  import { appWindow } from "skathi:app";
  import Extra from "./Extra.svelte";

  let count = $state(0);
  let palette = $state(false);
  let mini = $state(false);
  let shown = $state(0);
  let closed = $state(0);
  let openedId = $state(0);
  let boundId = $state(0);
  let size = $state("none");
  let pal: SkWindowExports | undefined = $state();
  let anchor: HTMLElement | undefined = $state();

  const width = $derived(appWindow.getBounds().width);
  const paletteTitle = $derived(`Palette - ${appWindow.title}`);

  const metrics = (m: SkWindowMetrics) => (size = `${Math.round(m.width)}x${Math.round(m.height)}`);
  const mapped = () => {
    const r = pal?.mapFromMain(anchor);
    return r ? `${r.width}x${r.height}` : "null";
  };
  const g = globalThis as { __SK_TEST_MAPPED__?: () => string; __SK_TEST_MOUNT_EXTRA__?: () => string };
  g.__SK_TEST_MAPPED__ = mapped;
  g.__SK_TEST_MOUNT_EXTRA__ = () => {
    mount(Extra, { target: anchor! });
    return "ok";
  };
  onDestroy(() => ((globalThis as { __SK_TEST_DESTROYED__?: string[] }).__SK_TEST_DESTROYED__ ??= []).push("app"));
</script>

<main>
  <p class="wtitle">{appWindow.title}</p>
  <p class="wwidth">{width}</p>
  <p class="meta">
    count <span class="count">{count}</span> · shown <span class="pal-shown">{shown}</span> · closed
    <span class="pal-closed">{closed}</span> · opened <span class="open-id">{openedId}</span> · bound
    <span class="pal-id">{boundId}</span> · size <span class="pal-size">{size}</span>
  </p>
  <div class="anchor" bind:this={anchor}></div>
  <button class="pop" onclick={() => (palette = true)}>pop out</button>
  <button class="back" onclick={() => (palette = false)}>bring it back</button>
  <button class="mini-size" onclick={() => (mini = !mini)}>toggle mini player</button>
  <button class="inc-a" onclick={() => count++}>+1</button>

  {#if palette}
    <SkWindow
      bind:this={pal}
      bind:windowId={boundId}
      class="pal-root"
      title={paletteTitle}
      bounds={{ x: 60, y: 470, width: 380, height: 320 }}
      alwaysOnTop
      minWidth={240}
      minHeight={mini ? 140 : 200}
      maxWidth={mini ? 320 : undefined}
      aspectRatio={mini ? 2 : undefined}
      opacity={mini ? 0.8 : 1}
      onopen={(id) => (openedId = id)}
      onshow={() => shown++}
      onresize={metrics}
      onclose={() => {
        palette = false;
        closed++;
      }}
    >
      <section class="panel">
        <span class="count-b">{count}</span>
        <span class="pwidth">{width}</span>
        <button class="inc-b" onclick={() => count++}>+1</button>
      </section>
    </SkWindow>
  {/if}
</main>

<style>
  main {
    padding: 16px;
  }
  .anchor {
    width: 50px;
    height: 20px;
  }
  .panel {
    padding: 12px;
  }
  .inc-b {
    width: 120px;
    height: 40px;
  }
</style>
