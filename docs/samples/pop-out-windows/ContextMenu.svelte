<script lang="ts">
  import { SkWindow, type SkWindowBounds, type SkWindowExports } from "@skathi-native/runtime/svelte";

  const actions = ["Cut", "Copy", "Paste"];
  let page = $state<HTMLElement>();
  let menu = $state<SkWindowExports>();
  let open = $state(false);
  let last = $state("nothing yet");
  let bounds = $state<SkWindowBounds>({ x: 100, y: 100, width: 160, height: 112 });

  function showAt(e: MouseEvent) {
    e.preventDefault();
    const r = menu?.mapFromMain(page) ?? null;
    if (!r || !page) return;
    const box = page.getBoundingClientRect();
    const { x = 0, y = 0 } = bounds;
    bounds = { ...bounds, x: x + r.x + e.clientX - box.left, y: y + r.y + e.clientY - box.top };
    open = true;
  }

  function pick(action: string) {
    last = action;
    open = false;
  }
</script>

<svelte:window oncontextmenu={showAt} onclick={() => (open = false)} />

<main bind:this={page} class="page">
  <p>Right-click anywhere. Last action: {last}.</p>
</main>
<SkWindow
  bind:this={menu}
  title="Menu"
  chrome="hidden"
  backdrop="transparent"
  resizable={false}
  focusable={false}
  taskbar={false}
  visible={open}
  {bounds}
>
  <div class="menu">
    {#each actions as a (a)}
      <button class="item" onclick={() => pick(a)}>{a}</button>
    {/each}
  </div>
</SkWindow>

<style>
  .page {
    height: 100%;
    padding: 24px;
  }
  .menu {
    display: flex;
    flex-direction: column;
    padding: 4px;
    border-radius: 8px;
    background: #1b1b1bf0;
    color: #ffffff;
  }
  .item {
    padding: 6px 12px;
    border-radius: 4px;
    text-align: left;
  }
  .item:hover {
    background: #0090ff;
  }
</style>
