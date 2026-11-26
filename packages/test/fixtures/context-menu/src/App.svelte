<script lang="ts">
  import {
    SkWindow,
    defineKeymap,
    SkContextMenu,
    SkKbd,
    SkMenuItem,
    SkMenuSeparator,
    SkSubmenu,
    type SkContextMenuHandle,
  } from "@skathi-native/runtime/svelte";
  import { onMount } from "svelte";
  let menu: SkContextMenuHandle<string> | undefined = $state();
  const files = ["Notes.txt", "Draft.txt"];
  let last = $state("");
  const shortcut = defineKeymap({ keys: "Control+KeyQ" });
  onMount(() => shortcut.bind(() => { last = "Shortcut fired"; }));
  onMount(() => shortcut.on("up", (event) => { if (event.cancelled) last = "Shortcut cancelled"; }));
  let extra = $state(false);
  let detached = $state(false);
  Object.assign(globalThis, {
    resizeMenu: () => (extra = !extra),
    showDetached: (value: boolean) => (detached = value),
    closeMenu: () => menu?.close(),
  });
</script>

<main class="files">
  {#each files as file (file)}
    <button oncontextmenu={(e) => menu?.open(e, file)} onclick={(e) => menu?.open(e.currentTarget, file)}>{file}</button
    >
  {/each}
  <output>{last}</output>
  <SkKbd id="layout-shortcut" map={shortcut} />
</main>
{#if detached}
  <SkWindow title="Detached files" bounds={{ width: 250, height: 140 }}>
    <button id="detached-trigger" onclick={(e) => menu?.open(e.currentTarget, "Detached.txt")}>Detached.txt</button>
  </SkWindow>
{/if}
<SkContextMenu bind:this={menu} aria-label="File actions" backdrop="acrylic">
  {#snippet children({ context: file })}
    <SkMenuItem onselect={() => (last = "Opened " + file)}>Open</SkMenuItem>
    <SkMenuItem disabled>Rename <SkKbd keys="F2" variant="plain" /></SkMenuItem>
    {#if extra}
      <SkMenuItem>
        Open in a separate application window
        <SkKbd keys={{ alternatives: [{ mouse: "middle" }, { gamepad: "south" }] }} gamepadLayout="playstation" />
      </SkMenuItem>
    {/if}
    <SkMenuSeparator />
    <SkSubmenu label="Copy">
      <SkMenuItem onselect={() => (last = "Copied " + file)}>File name</SkMenuItem>
    </SkSubmenu>
  {/snippet}
</SkContextMenu>

<style>
  .files {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 16px;
  }
</style>
