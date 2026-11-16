<script lang="ts">
  import { appWindow } from "@skathi-native/runtime/svelte";

  const STATE_EVENTS = ["maximize", "unmaximize", "minimize", "restore", "fullscreenchange", "move"];
  let log = $state<string[]>([]);
  let viewport = $state("");

  const push = (e: Event) => (log = [e.type, ...log].slice(0, 8));
  const onResize = (e: Event) => {
    viewport = `${Math.round(innerWidth)} x ${Math.round(innerHeight)} @ ${devicePixelRatio}x`;
    push(e);
  };

  $effect(() => {
    STATE_EVENTS.forEach((type) => appWindow.addEventListener(type, push));
    return () => STATE_EVENTS.forEach((type) => appWindow.removeEventListener(type, push));
  });
</script>

<svelte:window onresize={onResize} onfocus={push} onblur={push} />

<div class="panel">
  <p>{appWindow.isFocused() ? "Active" : "In the background"}, {viewport}</p>
  <ol class="log">
    {#each log as type, i (i)}
      <li>{type}</li>
    {/each}
  </ol>
</div>

<style>
  .panel {
    padding: 12px;
    font-size: 13px;
  }
  .log {
    margin-top: 8px;
    color: #ffffffb0;
  }
</style>
