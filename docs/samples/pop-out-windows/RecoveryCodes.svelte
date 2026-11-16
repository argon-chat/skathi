<script lang="ts">
  import { SkWindow } from "@skathi-native/runtime/svelte";

  let open = $state(false);
  let revealed = $state(false);
</script>

<main class="page">
  <button onclick={() => (open = true)}>Show recovery codes</button>
  {#if open}
    <SkWindow
      title="Recovery codes"
      bounds={{ width: 320, height: 220 }}
      contentProtection={revealed}
      onclose={() => (open = false)}
    >
      <div class="codes">
        {#if revealed}
          <p>4F7K-29QX · 8LMD-0RTE · ZC3V-71HN</p>
        {/if}
        <button onclick={() => (revealed = !revealed)}>{revealed ? "Hide" : "Reveal"}</button>
      </div>
    </SkWindow>
  {/if}
</main>

<style>
  .page,
  .codes {
    padding: 24px;
  }
</style>
