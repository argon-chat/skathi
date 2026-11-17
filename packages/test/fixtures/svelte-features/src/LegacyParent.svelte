<script lang="ts">
  import Legacy from "./Legacy.svelte";
  import { log } from "./log";
</script>

<Legacy start={5} on:bumped={(e) => log(`bumped:${e.detail}`)}>
  <span slot="extra" let:value id="slot">slot {value}</span>
</Legacy>
