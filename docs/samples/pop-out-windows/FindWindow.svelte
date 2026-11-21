<script lang="ts">
  import { SkWindow } from "@skathi-native/runtime/svelte";
  import { appWindow } from "skathi:app";

  let findOpen = $state(false);
  let findFocused = $state(false);
  let query = $state("");
</script>

<main class="page">
  <button onclick={() => (findOpen = true)}>Find</button>
  <p>Main window active: {appWindow.isFocused()}. Find window active: {findFocused}.</p>
  {#if findOpen}
    <SkWindow
      title="Find"
      bounds={{ width: 320, height: 120 }}
      taskbar={false}
      activate
      onfocus={() => (findFocused = true)}
      onblur={() => (findFocused = false)}
      onclose={() => (findOpen = false)}
    >
      <input bind:value={query} class="query" placeholder="Search the document" />
    </SkWindow>
  {/if}
</main>

<style>
  .page {
    padding: 24px;
  }
  .query {
    width: 100%;
    padding: 8px;
  }
</style>
