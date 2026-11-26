<script setup lang="ts">
import { defineKeymap, SkKbd, type HotkeyChord, type SkKeyMap } from "@skathi-native/runtime";
import { onScopeDispose, ref } from "vue";

defineProps<{ recorded?: HotkeyChord }>();
const find: SkKeyMap = defineKeymap({ keys: ["Mod", { code: "KeyF" }], scope: "app", suppress: true });
const presses = ref(0);
onScopeDispose(find.bind(() => presses.value++));
</script>

<template>
  <SkKbd :map="find" /> <output>{{ presses }}</output>
  <SkKbd keys="Mod+Shift+P" />
  <SkKbd :keys="{ sequence: ['Mod+K', 'Mod+C'] }" />
  <SkKbd :keys="{ alternatives: [['Mod', '+'], 'Mod+NumpadAdd'] }" />
  <SkKbd :keys="recorded" variant="plain" />
  <SkKbd :keys="['Mod', { mouse: 'wheel-up' }]" />
  <SkKbd :keys="{ alternatives: ['Enter', { gamepad: 'south' }] }" gamepad-layout="playstation" />
</template>
