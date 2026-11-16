<script setup lang="ts">
import { computed } from "vue";
import Icon from "../components/Icon.vue";
import Scenario from "../components/Scenario.vue";
import { draft, saveDraft } from "../store";

const dirty = computed(() => draft.text !== draft.saved);
</script>

<template>
  <Scenario id="closing/unsaved-changes">
    <div class="card editor">
      <div class="head">
        <Icon name="doc" />
        <span class="file">Review notes</span>
        <span class="status" :class="{ dirty }">{{ dirty ? "Edited" : "Saved" }}</span>
        <button class="btn" :disabled="!dirty" data-demo="save" @click="saveDraft">Save</button>
      </div>
      <textarea v-model="draft.text" class="text" data-demo="draft" />
    </div>
    <p class="muted">
      With the note saved, the close button closes the app at once. With edits, it asks first; Save and Don’t save both
      end with appWindow.close(), which does not ask again.
    </p>
  </Scenario>
</template>

<style scoped>
.editor {
  padding: 0;
  gap: 0;
  overflow: hidden;
}
.head {
  height: 52px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px 0 16px;
  border-bottom: 1px solid var(--card-line);
  color: var(--text-2);
}
.file {
  font-size: 14px;
  color: var(--text);
}
.status {
  flex: 1;
  font-size: 12px;
  color: var(--text-3);
}
.status.dirty {
  color: var(--accent-text);
}
.text {
  height: 260px;
  padding: 16px;
  background: transparent;
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
  line-height: 22px;
}
</style>
