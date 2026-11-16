<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { appWindow } from "@skathi-native/runtime";

const text = ref(localStorage.getItem("draft") ?? "");
const saved = ref(text.value);
const asking = ref(false);

function save() {
  localStorage.setItem("draft", text.value);
  saved.value = text.value;
}

// #region close
function onCloseRequested(e: Event) {
  if (text.value === saved.value) return;
  e.preventDefault();
  if (appWindow.isMinimized()) appWindow.restore();
  asking.value = true;
}

function answer(choice: "save" | "discard" | "cancel") {
  asking.value = false;
  if (choice === "cancel") return;
  if (choice === "save") save();
  appWindow.close();
}
// #endregion close

onMounted(() => appWindow.addEventListener("close-requested", onCloseRequested));
onUnmounted(() => appWindow.removeEventListener("close-requested", onCloseRequested));
</script>

<template>
  <div class="editor">
    <textarea v-model="text" class="text" />
    <div v-if="asking" class="scrim">
      <div class="sheet">
        <p>Save your changes before closing?</p>
        <div class="actions">
          <button class="btn primary" @click="answer('save')">Save</button>
          <button class="btn" @click="answer('discard')">Discard</button>
          <button class="btn" @click="answer('cancel')">Cancel</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor {
  position: relative;
  height: 100%;
  padding: 12px;
}
.text {
  width: 100%;
  height: 100%;
}
.scrim {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #00000066;
}
.sheet {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px;
  border-radius: 8px;
  background: #2b2b2b;
  color: #f3f3f3;
}
.actions {
  display: flex;
  gap: 8px;
}
.btn {
  padding: 6px 14px;
  border-radius: 6px;
  background: #ffffff14;
  color: #f3f3f3;
}
.btn.primary {
  background: #4cc2ff;
  color: #000000;
}
</style>
