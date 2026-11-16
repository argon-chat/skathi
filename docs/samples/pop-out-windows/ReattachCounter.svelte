<script lang="ts">
  import { SkWindow } from "@skathi-native/runtime/svelte";

  let count = $state(0);
</script>

<main class="page">
  <p>The counter opens in its own window. Close that window and the counter returns here.</p>
  <SkWindow title="Counter" bounds={{ width: 240, height: 160 }}>
    <div class="panel">
      <button onclick={() => count++}>Clicked {count} times</button>
    </div>
  </SkWindow>
</main>

<style>
  .page {
    padding: 24px;
  }
  .panel {
    padding: 16px;
  }
</style>
