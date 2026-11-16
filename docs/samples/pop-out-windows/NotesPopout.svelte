<script lang="ts">
  import { SkWindow, type SkWindowMetrics } from "@skathi-native/runtime/svelte";

  let notesOpen = $state(false);
  let note = $state("");
  let size = $state("");

  function onResize(m: SkWindowMetrics) {
    size = `${Math.round(m.width)} × ${Math.round(m.height)}`;
  }
</script>

<main class="page">
  <button onclick={() => (notesOpen = !notesOpen)}>{notesOpen ? "Close notes" : "Open notes"}</button>
  <p>{note.length} characters</p>
  <!-- #region window -->
  {#if notesOpen}
    <SkWindow
      title="Notes"
      bounds={{ width: 360, height: 280 }}
      activate
      onclose={() => (notesOpen = false)}
      onresize={onResize}
    >
      <textarea bind:value={note} class="notes"></textarea>
      <footer class="size">{size}</footer>
    </SkWindow>
  {/if}
  <!-- #endregion window -->
</main>

<style>
  .page {
    padding: 24px;
  }
  .notes {
    width: 100%;
    height: 200px;
    padding: 12px;
  }
  @media (max-width: 300px) {
    .size {
      display: none;
    }
  }
</style>
