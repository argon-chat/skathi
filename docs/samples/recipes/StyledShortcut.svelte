<script lang="ts">
  import { defineKeymap, SkKbd, type SkKbdLabels } from "@skathi-native/runtime/svelte";

  const confirm = defineKeymap({ keys: { alternatives: ["Mod+Enter", { gamepad: "south" }] } });
  const labels: SkKbdLabels = {
    keys: {
      Enter: { text: "↵", label: "Enter" },
      "Gamepad:south": { text: "✕", label: "Cross button" },
    },
  };
</script>

<div class="command-hints">
  <SkKbd map={confirm} {labels} />
</div>

<style>
  .command-hints :global(.sk-kbd) {
    font-family: "IBM Plex Mono", monospace;
    font-size: 14px;
    line-height: 20px;
  }
  .command-hints :global(.sk-kbd-key) {
    padding: 2px 7px;
    border: 1px solid #55647a;
    border-radius: 6px;
    background-color: #202938;
    color: #f3f6fa;
  }
  .command-hints :global(.sk-kbd-key[data-device="gamepad"]) :global(.sk-kbd-glyph) {
    display: none;
  }
</style>
