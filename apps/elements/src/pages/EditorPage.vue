<script setup lang="ts">
import { onMounted, ref } from "vue";

const empty = (): SkEditorValue => ({ text: "", marks: [], atoms: [] });
const draft = ref<SkEditorValue>(empty());
const editor = ref<SkEditorElement | null>(null);
const active = ref<string[]>([]);

const TYPOS = ["Fryday", "pallete", "teh"];

function spellcheck() {
  const ed = editor.value;
  if (!ed) return;
  const text = ed.slice(0);
  const hits: { from: number; to: number; name: string }[] = [];
  for (const m of text.matchAll(/\p{L}+/gu)) {
    if (TYPOS.includes(m[0]) && m.index + m[0].length < text.length) {
      hits.push({ from: m.index, to: m.index + m[0].length, name: "typo" });
    }
  }
  ed.setDecorations("spelling", hits);
}

function range(): [number, number] {
  const sel = editor.value?.selections[0];
  if (!sel) return [0, 0];
  return [Math.min(sel.anchor, sel.head), Math.max(sel.anchor, sel.head)];
}

function refresh() {
  const ed = editor.value;
  if (!ed) return;
  const [from, to] = range();
  active.value = from === to ? ed.typingMarks : ed.marksAt(to);
}

function toggle(type: string) {
  const ed = editor.value;
  if (!ed) return;
  const [from, to] = range();
  if (from === to) {
    ed.typingMarks = ed.typingMarks.includes(type)
      ? ed.typingMarks.filter((m) => m !== type)
      : [...ed.typingMarks, type];
  } else if (ed.marksAt(to).includes(type)) {
    ed.dispatch({ removeMarks: [{ type, from, to }] });
  } else {
    ed.dispatch({ addMarks: [{ type, from, to }] });
  }
  refresh();
}

let mentions = 0;
function mention() {
  const ed = editor.value;
  if (!ed) return;
  const [from, to] = range();
  const id = `mira-${++mentions}`;
  ed.dispatch({
    changes: [
      {
        from,
        to,
        insert: { text: "￼ ", marks: [], atoms: [{ id, at: 0, type: "mention", text: "@Mira" }] },
      },
    ],
  });
}

function onInput(e: Event) {
  draft.value = (e.target as SkEditorElement).value;
  spellcheck();
}

onMounted(() => {
  const ed = editor.value;
  if (!ed) return;
  ed.exclusiveMarks = ["link"];
  ed.addEventListener("change", () => {
    draft.value = ed.value;
    spellcheck();
  });
  ed.addEventListener("selectionchange", refresh);
});
</script>

<template>
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
              <div class="meta"><b>Jonah</b><span>10:42</span></div>
              <p>The new swatches are in the shared file. <b>Lilac</b> replaces the old violet everywhere.</p>
            </div>
          </div>
          <div class="msg">
            <span class="face c">S</span>
            <div class="body">
              <div class="meta"><b>Sana</b><span>10:44</span></div>
              <p>Looks great on the dark theme. Who signs off on it?</p>
            </div>
          </div>
          <div class="msg">
            <span class="face a">M</span>
            <div class="body">
              <div class="meta"><b>Mira</b><span>10:45</span></div>
              <p>Send it my way when it is ready.</p>
            </div>
          </div>
        </div>
        <div class="composer">
          <div class="tools" @mousedown.prevent>
            <button class="tool" :class="{ on: active.includes('bold') }" data-demo="bold" @click="toggle('bold')">
              <b>B</b>
            </button>
            <button class="tool" :class="{ on: active.includes('italic') }" @click="toggle('italic')">
              <i>I</i>
            </button>
            <button
              class="tool"
              :class="{ on: active.includes('accent') }"
              data-demo="accent"
              @click="toggle('accent')"
            >
              <span class="swatch" />
            </button>
            <button class="tool" :class="{ on: active.includes('link') }" @click="toggle('link')">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                <path
                  d="M6.5 9.5l3-3M7 4.5l1-1a2.8 2.8 0 014 4l-1 1M9 11.5l-1 1a2.8 2.8 0 01-4-4l1-1"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                />
              </svg>
            </button>
            <span class="sep" />
            <button class="tool at" data-demo="mention" @click="mention">@</button>
          </div>
          <div class="row">
            <sk-editor
              ref="editor"
              class="field"
              data-demo="editor"
              placeholder="Message #design"
              :value="draft"
              @input="onInput"
            >
              <span v-for="a in draft.atoms" :key="a.id" :atom="a.id" class="mention">
                <span class="dot">M</span>Mira
              </span>
            </sk-editor>
            <button class="send" @mousedown.prevent>
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                <path
                  d="M2.5 8h10M8.5 3.5L13 8l-4.5 4.5"
                  stroke="currentColor"
                  stroke-width="1.8"
                  stroke-linecap="round"
                />
              </svg>
            </button>
          </div>
        </div>
      </div>
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
.chat {
  width: 760px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: 0 24px 60px #0006;
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
  transition: background-color 120ms ease;
}
.tool:hover {
  background: #ffffff12;
}
.tool.on {
  background: var(--accent-bg);
  color: var(--pink);
}
.swatch {
  width: 14px;
  height: 14px;
  border-radius: 7px;
  background: var(--pink);
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
.field::selection {
  background-color: #c9a7f566;
}
.field::mark(bold) {
  font-weight: 700;
}
.field::mark(italic) {
  font-style: italic;
}
.field::mark(accent) {
  color: var(--pink);
  background: #f7a8d024;
  border-radius: 5px;
}
.field::mark(link) {
  color: var(--blue);
  text-decoration: underline;
}
.field::decoration(typo) {
  text-decoration: underline wavy #ff7a9c;
}
.mention {
  height: 22px;
  padding: 0 8px 0 3px;
  display: flex;
  align-items: center;
  gap: 5px;
  border-radius: 11px;
  background: #9fcdf52e;
  color: var(--blue);
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
  background: linear-gradient(135deg, #f7a8d0, #ec7fb6);
  color: var(--ink);
  font-size: 10px;
  font-weight: 700;
}
.send {
  width: 36px;
  height: 36px;
  border: none;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--pink);
  color: var(--ink);
}
</style>
