<script setup lang="ts">
import { ref } from "vue";

const draft = ref<SkEditorValue>({ text: "", marks: [], atoms: [] });
const tick = ref(0);
let ticking = false;

function step() {
  if (!ticking) return;
  tick.value++;
  requestAnimationFrame(step);
}

(globalThis as { __SK_TEST_JITTER__?: unknown }).__SK_TEST_JITTER__ = {
  start() {
    ticking = true;
    requestAnimationFrame(step);
  },
  stop() {
    ticking = false;
  },
  tick: () => tick.value,
};

function onInput(e: Event) {
  draft.value = (e.target as SkEditorElement).value;
}
</script>

<template>
  <div class="app">
    <div class="page">
      <div class="head">
        <span class="tag">&lt;sk-editor&gt;</span>
        <p>Rich text: marks, mentions as real elements, decorations, undo.</p>
      </div>
      <div class="stage">
        <div class="card chat">
          <div class="channel">
            <span class="hash">#</span>
            <b>design</b>
            <span class="topic">Palette and launch notes</span>
            <div class="faces">
              <span class="face a">M</span>
              <span class="face b">J</span>
              <span class="face c">S</span>
            </div>
          </div>
          <div class="messages">
            <div class="msg">
              <span class="face b">J</span>
              <div class="body">
                <div class="meta"><b>Jonah</b><span class="time">10:42</span></div>
                <p>The new swatches are in the shared file. <b>Lilac</b> replaces the old violet everywhere.</p>
              </div>
            </div>
            <div class="msg">
              <span class="face c">S</span>
              <div class="body">
                <div class="meta"><b>Sana</b><span class="time">10:44</span></div>
                <p>Looks great on the dark theme. Who signs off on it?</p>
              </div>
            </div>
            <div class="msg">
              <span class="face a">M</span>
              <div class="body">
                <div class="meta"><b>Mira</b><span class="time">10:45</span></div>
                <p>Send it my way when it is ready.</p>
              </div>
            </div>
          </div>
          <div class="composer">
            <div class="tools">
              <button class="tool"><b>B</b></button>
              <button class="tool"><i>I</i></button>
              <span class="sep" />
              <button class="tool at">@</button>
              <span id="tick" class="tick">{{ tick }}</span>
            </div>
            <div class="row">
              <sk-editor id="ed" class="field" placeholder="Message #design" :value="draft" @input="onInput" />
              <button class="send" />
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
:root {
  --font: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  --mono: "Cascadia Mono", Consolas, monospace;
  --text: #edeaf3;
  --body: #bcb8c9;
  --muted: #a09aad;
  --border: #ffffff14;
  --border-strong: #ffffff29;
  --accent-bg: #f7a8d024;
  --well: #0e0d13;
  --pink: #f7a8d0;
  --lilac: #c9a7f5;
  --ink: #1d1424;
}
.sk-window-root {
  position: relative;
}
.app {
  position: absolute;
  inset: 0;
  overflow: hidden;
  background: #17161e;
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
}
.page {
  position: absolute;
  inset: 40px 0 0 0;
  padding: 26px 56px 40px;
  display: flex;
  flex-direction: column;
  gap: 22px;
}
.head {
  display: flex;
  align-items: center;
  gap: 14px;
}
.tag {
  padding: 4px 10px;
  border-radius: 7px;
  background: var(--accent-bg);
  color: var(--pink);
  font-family: var(--mono);
  font-size: 15px;
}
.head p {
  margin: 0;
  color: var(--muted);
  font-size: 14px;
}
.card {
  border-radius: 18px;
  background: #211f2a;
  border: 1px solid var(--border);
}
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.chat {
  width: 760px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.channel {
  height: 56px;
  padding: 0 22px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid var(--border);
  font-size: 15px;
}
.hash {
  color: var(--pink);
  font-size: 18px;
}
.topic {
  margin-left: 8px;
  color: var(--muted);
  font-size: 13px;
}
.faces {
  margin-left: auto;
  display: flex;
  gap: 4px;
}
.face {
  width: 28px;
  height: 28px;
  border-radius: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ink);
  font-size: 12px;
  font-weight: 700;
}
.face.a {
  background: linear-gradient(135deg, #f7a8d0, #ec7fb6);
}
.face.b {
  background: linear-gradient(135deg, #9fcdf5, #c9a7f5);
}
.face.c {
  background: linear-gradient(135deg, #c9a7f5, #f7a8d0);
}
.messages {
  padding: 18px 22px 6px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.msg {
  display: flex;
  gap: 12px;
}
.msg .face {
  width: 34px;
  height: 34px;
  border-radius: 17px;
  flex-shrink: 0;
}
.body {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.meta {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 13px;
}
.meta span {
  color: var(--muted);
  font-size: 12px;
}
.body p {
  margin: 0;
  color: var(--body);
  line-height: 21px;
}
.body p b {
  color: var(--lilac);
}
.composer {
  margin: 14px 18px 18px;
  border-radius: 14px;
  border: 1px solid var(--border-strong);
  background: var(--well);
}
.tools {
  height: 40px;
  padding: 0 8px;
  display: flex;
  align-items: center;
  gap: 2px;
  border-bottom: 1px solid var(--border);
}
.tool {
  width: 32px;
  height: 28px;
  border: none;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  color: var(--body);
  font-family: var(--font);
  font-size: 14px;
}
.sep {
  width: 1px;
  height: 18px;
  margin: 0 6px;
  background: var(--border-strong);
}
.at {
  font-weight: 600;
}
.tick {
  margin-left: auto;
  color: var(--muted);
  font-size: 12px;
}
.row {
  display: flex;
  align-items: flex-end;
  gap: 10px;
  padding: 10px 10px 10px 16px;
}
.field {
  flex: 1;
  min-height: 24px;
  max-height: 140px;
  padding: 2px 0;
  color: var(--text);
  font-size: 15px;
  line-height: 24px;
}
.field::placeholder {
  color: var(--muted);
}
.send {
  width: 36px;
  height: 36px;
  border: none;
  border-radius: 10px;
  background: var(--pink);
}
</style>
