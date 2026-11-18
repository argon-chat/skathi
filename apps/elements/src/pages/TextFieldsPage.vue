<script setup lang="ts">
import { ref } from "vue";

const title = ref("");
const notes = ref("");

function clear() {
  title.value = "";
  notes.value = "";
  (document.activeElement as HTMLElement | null)?.blur?.();
}
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;input&gt;</span>
      <span class="tag">&lt;textarea&gt;</span>
      <p>Native text editing: typing, selection with the keyboard, undo.</p>
    </div>
    <div class="stage">
      <form class="card form" @submit.prevent>
        <div class="row">
          <span class="date">
            <b>24</b>
            <small>MAY</small>
          </span>
          <div class="title">New event</div>
        </div>
        <label class="field">
          <span class="lbl"
            >Title <i>{{ title.length }} / 40</i></span
          >
          <input v-model="title" class="input" data-demo="title" maxlength="40" placeholder="What's happening?" />
        </label>
        <label class="field">
          <span class="lbl"
            >Notes <i>{{ notes.length }} chars</i></span
          >
          <textarea v-model="notes" class="input area" data-demo="notes" rows="4" placeholder="Anything to bring?" />
        </label>
        <div class="actions">
          <button type="button" class="btn" data-demo="cancel" @click="clear">Cancel</button>
          <button type="button" class="btn primary">Save event</button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.form {
  width: 620px;
  padding: 28px 32px;
  display: flex;
  flex-direction: column;
  gap: 20px;
}
.row {
  display: flex;
  align-items: center;
  gap: 14px;
}
.date {
  width: 52px;
  height: 52px;
  border-radius: 14px;
  background: var(--accent-bg);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  line-height: 1;
}
.date b {
  font-size: 20px;
  color: var(--pink);
}
.date small {
  font-size: 10px;
  letter-spacing: 0.1em;
  color: var(--muted);
}
.title {
  font-family: var(--font-display);
  font-size: 22px;
  font-weight: 600;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.lbl {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
  color: var(--body);
}
.lbl i {
  font-style: normal;
  font-family: var(--mono);
  font-size: 12px;
  color: var(--muted);
}
.input {
  height: 44px;
  padding: 0 14px;
  border-radius: 11px;
  border: 1px solid var(--border-strong);
  background: var(--well);
  color: var(--text);
  font-family: var(--font);
  font-size: 16px;
  caret-color: var(--pink);
  outline: none;
  transition:
    border-color 140ms ease,
    box-shadow 140ms ease;
}
.input::placeholder {
  color: #6f6880;
}
.input::selection {
  background: #624057;
}
.input:focus {
  border-color: var(--pink);
  box-shadow: 0 0 0 3px #f7a8d033;
}
.area {
  height: auto;
  min-height: 120px;
  padding: 11px 14px;
  line-height: 1.5;
  resize: none;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>
