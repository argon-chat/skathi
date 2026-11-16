<script setup lang="ts">
import { answerClose } from "../closing";
</script>

<template>
  <div class="scrim">
    <div class="sheet">
      <div class="body">
        <h2 class="title">Save changes to “Review notes”?</h2>
        <p class="text">The note has edits that are not saved. If you close without saving, they are lost.</p>
      </div>
      <div class="actions">
        <button class="btn accent" data-demo="sheet-save" @click="answerClose('save')">Save</button>
        <button class="btn" data-demo="sheet-discard" @click="answerClose('discard')">Don’t save</button>
        <button class="btn" data-demo="sheet-cancel" @click="answerClose('cancel')">Cancel</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--scrim);
  z-index: 100;
}
.sheet {
  width: 440px;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  border: 1px solid var(--control-line);
  background: var(--sheet);
  box-shadow: 0 16px 40px var(--shadow);
  overflow: hidden;
}
.body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 24px;
}
.title {
  margin: 0;
  font-family: var(--font-display);
  font-size: 20px;
  font-weight: 600;
  color: var(--text);
}
.text {
  margin: 0;
  font-size: 14px;
  line-height: 20px;
  color: var(--text-2);
}
.actions {
  display: flex;
  gap: 8px;
  padding: 24px;
  border-top: 1px solid var(--card-line);
  background: var(--well);
}
.actions .btn {
  flex: 1;
}
</style>
