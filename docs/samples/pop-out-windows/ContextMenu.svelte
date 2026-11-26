<script lang="ts">
  import {
    SkContextMenu,
    SkMenuItem,
    SkMenuSeparator,
    SkSubmenu,
    type SkContextMenuHandle,
  } from "@skathi-native/runtime/svelte";
  let menu: SkContextMenuHandle<string> | undefined = $state();
  const files = ["Notes.txt", "Draft.txt"];
  let last = $state("");
</script>

<main class="files">
  {#each files as file (file)}
    <button oncontextmenu={(e) => menu?.open(e, file)} onclick={(e) => menu?.open(e.currentTarget, file)}>{file}</button
    >
  {/each}
  <output>{last}</output>
</main>
<SkContextMenu bind:this={menu} aria-label="File actions" backdrop="acrylic">
  {#snippet children({ context: file })}
    <SkMenuItem onselect={() => (last = "Opened " + file)}>Open</SkMenuItem>
    <SkMenuItem disabled>Rename</SkMenuItem>
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
