<script setup lang="ts">
import { ref } from "vue";

defineProps<{ tick: number }>();

const draft = ref<SkEditorValue>({
  text: "Ask ￼ about it",
  marks: [],
  atoms: [{ id: "m1", at: 4, type: "mention", text: "@Mira" }],
});

function onInput(e: Event) {
  draft.value = (e.target as SkEditorElement).value;
}
</script>

<template>
  <div class="atom-page">
    <div class="atom-card">
      <span id="tick" class="atom-tick">{{ tick }}</span>
      <sk-editor id="aed" class="atom-field" :value="draft" @input="onInput">
        <span v-for="a in draft.atoms" :key="a.id" :atom="a.id" class="mention"> <span class="dot">M</span>Mira </span>
      </sk-editor>
    </div>
  </div>
</template>

<style>
.atom-page {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.atom-card {
  width: 517px;
  padding: 13px;
  display: flex;
  flex-direction: column;
  gap: 7px;
  border: 1px solid var(--border);
}
.atom-tick {
  font-size: 12px;
  color: var(--muted);
}
.atom-field {
  color: var(--text);
  font-size: 15px;
  line-height: 24px;
}
.mention {
  height: 22px;
  padding: 0 8px 0 3px;
  display: flex;
  align-items: center;
  gap: 5px;
  border-radius: 11px;
  background: #9fcdf52e;
  color: #9fcdf5;
  font-size: 14px;
  font-weight: 600;
  vertical-align: middle;
}
.dot {
  width: 16px;
  height: 16px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #f7a8d0;
  color: var(--ink);
  font-size: 10px;
  font-weight: 700;
}
</style>
