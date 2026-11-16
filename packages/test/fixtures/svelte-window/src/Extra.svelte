<script lang="ts">
  import { onDestroy } from "svelte";

  onDestroy(() => ((globalThis as { __SK_TEST_DESTROYED__?: string[] }).__SK_TEST_DESTROYED__ ??= []).push("extra"));
</script>

<span class="extra">extra</span>
