<script lang="ts">
  import { defineKeymap, SkKbd, type HotkeyChord, type SkKeyMap } from "@skathi-native/runtime/svelte";
  import { onMount } from "svelte";

  let { recorded }: { recorded?: HotkeyChord } = $props();
  const find: SkKeyMap = defineKeymap({ keys: ["Mod", { code: "KeyF" }], scope: "app", suppress: true });
  let presses = $state(0);
  onMount(() => find.bind(() => presses++));
</script>

<SkKbd map={find} /> <output>{presses}</output>
<SkKbd keys="Mod+Shift+P" />
<SkKbd keys={{ sequence: ["Mod+K", "Mod+C"] }} />
<SkKbd keys={{ alternatives: [["Mod", "+"], "Mod+NumpadAdd"] }} />
<SkKbd keys={recorded} variant="plain" />
<SkKbd keys={["Mod", { mouse: "wheel-up" }]} />
<SkKbd keys={{ alternatives: ["Enter", { gamepad: "south" }] }} gamepadLayout="playstation" />
