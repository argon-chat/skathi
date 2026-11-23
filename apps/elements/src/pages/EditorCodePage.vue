<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

const LINES = 100_000;
const grouped = (n: number) => String(n).replace(/\B(?=(\d{3})+$)/g, ",");
const HEAD = `// Frames of a looping animation, one every 1/60 s, kept in a table.
export interface Frame {
	index: number;
	at: number;
	hue: number;
	label: string;
}

const RATE = 60;
const PERIOD = 4;

export const describe = (f: Frame) => \`\${f.label} at \${f.at.toFixed(3)} s, hue \${Math.round(f.hue * 360)} degrees, one of a table generated once at build time so that the player never computes a frame while it runs\`;

export function frameAt(index: number): Frame {
	const at = index / RATE;
	const hue = (at / PERIOD) % 1;
	return { index, at, hue, label: \`frame \${index}\` };
}

export function nearest(frames: Frame[], time: number): Frame | undefined {
	let lo = 0;
	let hi = frames.length - 1;
	while (lo < hi) {
		const mid = (lo + hi) >> 1;
		if (frames[mid]!.at < time) lo = mid + 1;
		else hi = mid;
	}
	return frames[lo];
}

export const frames: Frame[] = [`;

function source(): string {
  const head = HEAD.split("\n");
  const rows = LINES - head.length - 1;
  const out = head;
  for (let i = 0; i < rows; i++) {
    const at = (i / 60).toFixed(4);
    const hue = ((i / 240) % 1).toFixed(4);
    out.push(`\t{ index: ${i}, at: ${at}, hue: ${hue}, label: "frame ${i}" },`);
  }
  out.push("];");
  return out.join("\n");
}

const KEYWORDS = new Set([
  "export",
  "interface",
  "const",
  "let",
  "function",
  "return",
  "while",
  "if",
  "else",
  "undefined",
]);
const TOKEN = /(\/\/.*$)|(`[^`]*`|"[^"]*")|(\b\d+(?:\.\d+)?\b)|([A-Za-z_]\w*)(?=\s*\()|([A-Za-z_]\w*)/g;

const editor = ref<SkEditorElement | null>(null);
const query = ref("");
const findOpen = ref(false);
const caret = ref({ line: 1, col: 1 });
const focused = ref(false);
const total = ref(0);
let text = "";

const status = computed(() => `Ln ${grouped(caret.value.line)}, Col ${caret.value.col}`);
const count = computed(() => (query.value ? `${grouped(total.value)} matches` : ""));

function decorate() {
  const ed = editor.value;
  if (!ed) return;
  const { fromLine, toLine } = ed.viewport;
  const syntax: { from: number; to: number; name: string }[] = [];
  const hits: { from: number; to: number; name: string }[] = [];
  const q = query.value;
  for (let n = Math.max(0, fromLine - 5); n < Math.min(ed.lineCount, toLine + 5); n++) {
    const line = ed.line(n);
    for (const m of line.text.matchAll(TOKEN)) {
      const from = line.from + m.index;
      const to = from + m[0].length;
      const name = m[1]
        ? "comment"
        : m[2]
          ? "string"
          : m[3]
            ? "number"
            : m[4]
              ? "call"
              : KEYWORDS.has(m[0])
                ? "keyword"
                : /^[A-Z]/.test(m[0])
                  ? "type"
                  : "";
      if (name) syntax.push({ from, to, name });
    }
    if (q) {
      for (let at = line.text.indexOf(q); at >= 0; at = line.text.indexOf(q, at + q.length)) {
        hits.push({ from: line.from + at, to: line.from + at + q.length, name: "match" });
      }
    }
  }
  ed.setDecorations("syntax", syntax);
  ed.setDecorations("search", hits);
}

function countMatches() {
  const q = query.value;
  let n = 0;
  if (q) for (let at = text.indexOf(q); at >= 0; at = text.indexOf(q, at + q.length)) n++;
  total.value = n;
}

function onQuery(e: Event) {
  query.value = (e.target as HTMLInputElement).value;
  countMatches();
  decorate();
}

function onFindKey(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  query.value = "";
  (e.target as HTMLInputElement).value = "";
  countMatches();
  decorate();
}

function lineDecorations() {
  const ed = editor.value;
  if (!ed) return;
  const head = ed.selections[0]?.head ?? 0;
  const line = ed.lineAt(head);
  caret.value = { line: line + 1, col: head - ed.line(line).from + 1 };
  ed.setLineDecorations("caret", focused.value ? [{ line, name: "current" }] : []);
}

let raf = 0;
onMounted(() => {
  const ed = editor.value;
  if (!ed) return;
  text = source();
  ed.value = { text, marks: [], atoms: [] };
  ed.dispatch({ selections: [{ anchor: 0 }], scrollIntoView: true });
  ed.setLineDecorations("diff", [
    { line: 8, name: "added", gutter: "+" },
    { line: 9, name: "added", gutter: "+" },
    { line: 15, name: "changed", gutter: "~" },
    { line: 16, name: "changed", gutter: "~" },
  ]);
  ed.setLineDecorations("marks", [{ line: 24, name: "stop", gutter: "●" }]);
  ed.addEventListener("keydown", (e) => {
    const key = e as KeyboardEvent;
    if (!key.ctrlKey || (key.key !== "Home" && key.key !== "End")) return;
    key.preventDefault();
    ed.dispatch({ selections: [{ anchor: key.key === "Home" ? 0 : ed.length }], scrollIntoView: true });
  });
  ed.addEventListener("viewportchange", decorate);
  ed.addEventListener("selectionchange", lineDecorations);
  ed.addEventListener("focus", () => {
    focused.value = true;
    lineDecorations();
  });
  ed.addEventListener("blur", () => {
    focused.value = false;
    lineDecorations();
  });
  raf = requestAnimationFrame(decorate);
});
onBeforeUnmount(() => cancelAnimationFrame(raf));
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;sk-editor&gt;</span>
      <p>A 100,000-line document with a gutter, decorations and no wrapping.</p>
    </div>
    <div class="stage">
      <div class="card ide">
        <div class="tabs">
          <span class="file on">
            <span class="ts">TS</span>
            frames.ts
            <span class="dirty" />
          </span>
          <span class="file">player.ts</span>
          <div class="find" :class="{ open: findOpen || query }">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
              <circle cx="7" cy="7" r="4.5" stroke="currentColor" stroke-width="1.6" />
              <path d="M10.5 10.5L14 14" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
            </svg>
            <input
              data-demo="find"
              placeholder="Find"
              spellcheck="false"
              @input="onQuery"
              @keydown="onFindKey"
              @focus="findOpen = true"
              @blur="findOpen = false"
            />
            <span class="count">{{ count }}</span>
          </div>
        </div>
        <sk-editor ref="editor" class="code" data-demo="code" />
        <div class="status">
          <span class="branch">
            <svg width="12" height="12" viewBox="0 0 16 16" fill="none">
              <circle cx="4" cy="3.5" r="2" stroke="currentColor" stroke-width="1.5" />
              <circle cx="4" cy="12.5" r="2" stroke="currentColor" stroke-width="1.5" />
              <circle cx="12" cy="5" r="2" stroke="currentColor" stroke-width="1.5" />
              <path d="M4 5.5v5M12 7c0 3-8 2-8 3.5" stroke="currentColor" stroke-width="1.5" />
            </svg>
            main
          </span>
          <span class="added">+2</span>
          <span class="changed">~2</span>
          <span class="spacer" />
          <span data-demo="status">{{ status }}</span>
          <span>{{ grouped(LINES) }} lines</span>
          <span>Tab size 4</span>
          <span>TypeScript</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.stage {
  flex: 1;
  display: flex;
  min-height: 0;
}
.ide {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: 0 24px 60px #0006;
}
.tabs {
  height: 44px;
  padding: 0 10px 0 8px;
  display: flex;
  align-items: center;
  gap: 4px;
  border-bottom: 1px solid var(--border);
}
.file {
  height: 30px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 8px;
  color: var(--muted);
  font-size: 13px;
}
.file.on {
  background: #ffffff0d;
  color: var(--text);
}
.ts {
  padding: 1px 4px;
  border-radius: 4px;
  background: #9fcdf52a;
  color: var(--blue);
  font-size: 10px;
  font-weight: 700;
}
.dirty {
  width: 7px;
  height: 7px;
  border-radius: 4px;
  background: var(--pink);
}
.find {
  margin-left: auto;
  width: 300px;
  height: 30px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--well);
  color: var(--muted);
  transition: border-color 120ms ease;
}
.find.open {
  border-color: #f7a8d080;
}
.find input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  color: var(--text);
  font-family: var(--mono);
  font-size: 13px;
}
.count {
  color: var(--muted);
  font-size: 12px;
}
.code {
  flex: 1;
  min-height: 0;
  padding: 10px 0;
  color: #d9d4e6;
  font-family: var(--mono);
  font-size: 14px;
  line-height: 22px;
  white-space: pre;
  tab-size: 4;
}
.code::gutter {
  padding: 0 14px 0 18px;
  min-width: 92px;
  border-right: 1px solid var(--border);
  color: #6f6a80;
}
.code::line-number {
  color: #6f6a80;
}
.code::gutter(added) {
  color: #8fdcb0;
  background: #7fd6a433;
}
.code::gutter(changed) {
  color: var(--blue);
  background: #9fcdf52e;
}
.code::gutter(stop) {
  color: var(--pink-deep);
}
.code::gutter(current) {
  color: var(--text);
}
.code::decoration(added) {
  background: #7fd6a41f;
}
.code::decoration(changed) {
  background: #9fcdf51c;
}
.code::decoration(current) {
  background: #ffffff0a;
}
.code::decoration(keyword) {
  color: var(--lilac);
}
.code::decoration(type) {
  color: var(--blue);
}
.code::decoration(call) {
  color: var(--pink);
}
.code::decoration(string) {
  color: #a8e0bd;
}
.code::decoration(number) {
  color: #f5c98f;
}
.code::decoration(comment) {
  color: #7d778c;
}
.code::decoration(match) {
  background: #f7a8d04d;
  border-radius: 4px;
}
.code::selection {
  background-color: #c9a7f540;
}
.status {
  height: 30px;
  padding: 0 16px;
  display: flex;
  align-items: center;
  gap: 18px;
  border-top: 1px solid var(--border);
  color: var(--muted);
  font-size: 12px;
}
.branch {
  display: flex;
  align-items: center;
  gap: 6px;
}
.added {
  color: #8fdcb0;
}
.changed {
  color: var(--blue);
}
.spacer {
  flex: 1;
}
</style>
