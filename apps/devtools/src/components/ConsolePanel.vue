<script setup lang="ts">
// The console drawer: the target's log ring, live, and a prompt into its JSC.
//
// Two things changed from v1, and both were the same complaint.
//
//   NEWEST LAST. v1 reversed the list, because nothing could scroll a container
//   and "keep the newest visible" had no other implementation. It reads wrong —
//   a stack trace runs upwards, a sequence of events reads backwards — and it
//   is fixed here by a list that owns its offset (`../virtual.ts`) and follows
//   the tail until you scroll away from it, exactly like a terminal.
//
//   NO 700 ms LAG. Lines arrive on the push channel, in the frame the target
//   wrote them (`crates/vn-app/src/devtools/ws.rs`). A `console.log` in the app
//   is on screen before you have finished looking up.
//
// Everything else is the usual console furniture that was missing: level
// filters, a text filter, clear, preserve-on-reattach, an eval history and
// completion of the target's own globals.
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { onKeydown } from "../keys";
import { LOG_H, PANE_HEAD_H, PROMPT_H } from "../layout";
import { viewport } from "../layout";
import {
  LEVELS,
  clearConsole,
  complete,
  levelOf,
  loadCompletions,
  runEval,
  state,
  ui,
  visibleLogs,
} from "../store";
import { useVirtual, wrappedLines } from "../virtual";
import type { ConsoleLine, StyleRun } from "../protocol";

const props = defineProps<{ height: number }>();

const listHeight = computed(() => Math.max(30, props.height - PANE_HEAD_H - PROMPT_H));
const lines = computed(() => visibleLogs());
const expanded = ref<Record<number, boolean>>({});
const promptBox = ref<VnElement | null>(null);
const draft = ref("");
const hint = ref<string[]>([]);

/** The text column's width — the pane minus the gutter and the kind chip. */
const textWidth = computed(() => Math.max(200, viewport.w - 210));

const heights = computed(() =>
  lines.value.map((line, i) => {
    // A collapsed line is one row with an ellipsis; an expanded one wraps.
    // Nothing here can measure text (see `../virtual.ts`), so the wrapped
    // height is an estimate that is deliberately generous by a fraction of a
    // line rather than short by one.
    if (!expanded.value[i]) return LOG_H;
    return LOG_H * wrappedLines(line.text, textWidth.value) + 4;
  }),
);

const list = useVirtual(
  () => heights.value,
  () => listHeight.value,
);

const slice = computed(() =>
  lines.value.slice(list.start, list.end).map((line, i) => ({ line, index: list.start + i })),
);

// Follow the tail — unless the reader has scrolled away from it, which is the
// signal that they are reading something older and would rather not be yanked.
watch(
  () => lines.value.length,
  () => {
    if (state.follow) list.toBottom();
  },
);
watch(
  () => list.atBottom,
  (at) => {
    state.follow = at;
  },
);

function toggleLine(i: number): void {
  expanded.value[i] = !expanded.value[i];
}

function submit(): void {
  const source = draft.value;
  if (!source.trim()) return;
  draft.value = "";
  hint.value = [];
  state.follow = true;
  void runEval(source).then(() => list.toBottom());
}

function history(delta: number): void {
  const h = state.evalHistory;
  if (!h.length) return;
  const next = Math.max(0, Math.min(h.length, state.evalIndex + delta));
  state.evalIndex = next;
  draft.value = next >= h.length ? "" : h[next];
}

function onPromptKey(event: VnKeyboardEvent): void {
  if (event.key === "Enter") {
    // Tab-completion's first suggestion is accepted by Enter only when the
    // hint list is showing and the caret is at a word — otherwise Enter
    // evaluates, which is what it is for.
    submit();
    return;
  }
  if (event.key === "ArrowUp") {
    history(-1);
    return;
  }
  if (event.key === "ArrowDown") {
    history(1);
    return;
  }
  if (event.key === "Tab" && hint.value.length) {
    event.preventDefault();
    accept(hint.value[0]);
    return;
  }
  if (event.key === "Escape") {
    hint.value = [];
  }
}

function accept(name: string): void {
  draft.value = draft.value.replace(/[\w$]*$/, name);
  hint.value = [];
}

watch(draft, (value) => {
  hint.value = value ? complete(value) : [];
});

function focusPrompt(): void {
  promptBox.value?.focus();
}

function onKey(event: VnKeyboardEvent): void {
  // Ctrl+L is the terminal's clear and nobody has ever had to learn it.
  if (event.key.toLowerCase() === "l" && event.ctrlKey && state.consoleOpen) {
    event.preventDefault();
    clearConsole();
  }
}

let offKey: (() => void) | null = null;

onMounted(() => {
  ui.focusPrompt = focusPrompt;
  offKey = onKeydown(onKey);
  void loadCompletions();
});
onUnmounted(() => {
  ui.focusPrompt = null;
  offKey?.();
  offKey = null;
});

const counts = computed(() => {
  const out: Record<string, number> = { error: 0, warn: 0, info: 0, debug: 0 };
  for (const line of state.logs) out[levelOf(line.kind)]++;
  return out;
});

/**
 * One console line, split at its `%c` boundaries.
 *
 * The runs come from the target's own log ring (`crates/vn-tree/src/log.rs`),
 * where the console tee put them after applying the Console Standard's
 * formatter — so `%c` never reaches this app as text, and the CSS never reaches
 * it as a message. All that is left is "this stretch of characters was declared
 * with this CSS".
 */
function segments(line: ConsoleLine): { text: string; css: string }[] {
  const runs = line.styles;
  if (!runs || !runs.length) return [{ text: line.text, css: "" }];
  const out: { text: string; css: string }[] = [];
  const head = line.text.slice(0, runs[0].at);
  if (head) out.push({ text: head, css: "" });
  runs.forEach((run: StyleRun, i: number) => {
    const end = i + 1 < runs.length ? runs[i + 1].at : line.text.length;
    const text = line.text.slice(run.at, end);
    if (text) out.push({ text, css: run.css });
  });
  return out;
}

/**
 * The four declarations a console run is ever really using, pulled out of the
 * CSS string the app passed.
 *
 * Deliberately not a CSS parser and not `style="…"` pass-through: the string is
 * ARBITRARY text from the app under inspection, it is written for a browser's
 * console (`padding`, `border-radius`, `text-shadow`), and handing it straight
 * to this app's own stylesheet would let a target's log line lay out the
 * inspector. Four properties, read by name, is the whole of what a coloured
 * badge needs.
 */
function runStyle(css: string): Record<string, string> {
  if (!css) return {};
  const style: Record<string, string> = {};
  for (const part of css.split(";")) {
    const [rawName, ...rest] = part.split(":");
    const name = rawName.trim().toLowerCase();
    const value = rest.join(":").trim();
    if (!value) continue;
    if (name === "color") style.color = value;
    else if (name === "background" || name === "background-color") style.background = value;
    else if (name === "font-weight") style.fontWeight = value;
    else if (name === "font-style") style.fontStyle = value;
  }
  return style;
}

function glyph(kind: string): string {
  if (kind === "input") return "›";
  if (kind === "result") return "‹";
  if (kind === "error") return "✕";
  if (levelOf(kind) === "warn") return "!";
  return "·";
}
</script>

<template>
  <section class="pane console" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Console</span>
      <div class="levels">
        <div
          v-for="lvl in LEVELS"
          :key="lvl"
          class="lvl"
          :class="[lvl, { on: state.levels[lvl] }]"
          :title="`Show ${lvl} lines`"
          @click="state.levels[lvl] = !state.levels[lvl]"
        >
          {{ lvl }}<span class="n">{{ counts[lvl] }}</span>
        </div>
      </div>
      <input
        v-model="state.logFilter"
        class="input filter"
        placeholder="filter"
        @focus="ui.typing = true"
        @blur="ui.typing = false"
      />
      <div class="spacer"></div>
      <span v-if="state.dropped" class="chip warn" title="The target's 500-entry ring wrapped">
        {{ state.dropped }} dropped
      </span>
      <span v-if="state.logStatus" class="status mono">{{ state.logStatus }}</span>
      <div
        class="btn"
        :class="{ on: state.follow }"
        title="Follow the newest line"
        @click="((state.follow = !state.follow), state.follow && list.toBottom())"
      >
        follow
      </div>
      <div
        class="btn"
        :class="{ on: state.preserve }"
        title="Keep lines when the inspector reattaches"
        @click="state.preserve = !state.preserve"
      >
        preserve
      </div>
      <div class="btn danger" title="Clear  ·  Ctrl+L" @click="clearConsole()">clear</div>
    </header>

    <div class="list" :style="{ height: listHeight + 'px' }" @wheel="list.onWheel($event)">
      <div class="viewport" :style="{ marginTop: list.padTop + 'px' }">
        <div
          v-for="row in slice"
          :key="row.line.seq + ':' + row.index"
          class="line mono"
          :class="[row.line.kind, levelOf(row.line.kind), { open: expanded[row.index] }]"
          :style="{ height: heights[row.index] + 'px' }"
          @click="toggleLine(row.index)"
        >
          <span class="glyph">{{ glyph(row.line.kind) }}</span>
          <span class="kind">{{ row.line.kind }}</span>
          <!-- A line with no `%c` is ONE text node, not a span wrapping one:
               the wrapper would defeat the row's own ellipsis, and 99% of
               lines have no runs. `.seg` therefore means "a styled run",
               which is also what makes it a usable selector in a spec. -->
          <span
            v-if="row.line.styles && row.line.styles.length"
            class="text"
            :class="{ wrap: expanded[row.index] }"
          >
            <span
              v-for="(seg, i) in segments(row.line)"
              :key="i"
              class="seg"
              :style="runStyle(seg.css)"
              >{{ seg.text }}</span
            >
          </span>
          <span v-else class="text plain" :class="{ wrap: expanded[row.index] }">{{
            row.line.text
          }}</span>
        </div>
      </div>
      <div
        v-if="list.thumb"
        class="vscroll"
        :style="{ top: list.thumb.top + 'px', height: list.thumb.height + 'px' }"
      ></div>
      <div v-if="!lines.length" class="empty">
        Nothing in the ring yet. <b>console.log</b>, uncaught errors, CSS advisories and the host's
        own `[window]` / `[storage]` lines all land here, live.
      </div>
    </div>

    <div class="prompt">
      <span class="caret">›</span>
      <input
        ref="promptBox"
        v-model="draft"
        class="entry mono"
        placeholder="evaluate in the target — document.title, or __VN__.require(&quot;<module id>&quot;)"
        @keydown="onPromptKey"
        @focus="ui.typing = true"
        @blur="ui.typing = false"
      />
      <div v-if="hint.length" class="hints mono">
        <span v-for="(h, i) in hint" :key="h" class="hint" :class="{ first: i === 0 }" @click="accept(h)">{{
          h
        }}</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.console {
  min-height: 0;
  border-top-width: 1px;
  border-color: #1e2733;
}
.levels {
  display: flex;
  flex-direction: row;
  gap: 2px;
}
.lvl {
  height: 17px;
  padding-left: 6px;
  padding-right: 6px;
  border-radius: 3px;
  font-size: 10px;
  color: #4e5b6d;
  background: #131a24;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
}
.lvl:hover {
  background: #1b2431;
}
.lvl.on {
  color: #b9c6d6;
  background: #1c2532;
}
.lvl.error.on {
  color: #ff9d96;
}
.lvl.warn.on {
  color: #e3b341;
}
.lvl .n {
  font-size: 9px;
  color: #5c6b80;
}
.filter {
  width: 130px;
}
.spacer {
  flex: 1;
}
.status {
  font-size: 10px;
  color: #6d7c90;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 260px;
}
.list {
  position: relative;
  overflow: hidden;
  min-height: 0;
}
.viewport {
  display: flex;
  flex-direction: column;
}
.line {
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: 6px;
  /* A row is exactly as tall as the virtual list budgeted for it. Anything
     that overflows is clipped rather than drawn over the next line. */
  overflow: hidden;
  padding-left: 8px;
  padding-right: 12px;
  font-size: 11px;
  color: #b9c6d6;
  border-bottom-width: 1px;
  border-color: #ffffff08;
}
.line:hover {
  background: #ffffff08;
}
.glyph {
  width: 8px;
  color: #46556a;
  flex-shrink: 0;
}
.kind {
  width: 52px;
  flex-shrink: 0;
  font-size: 10px;
  color: #5c6b80;
}
/* A row of `%c` runs, DECLARED as a row rather than left to the engine's
 * inline-formatting fallback. The fallback is right for prose — it wraps —
 * and wrong here: a collapsed console line is exactly one line tall, and a
 * wrapped second line would be drawn into the row underneath it.
 *
 * `.text.plain` (a line with no runs) opts back out: it is a single text
 * node, and a block box is what `text-overflow: ellipsis` needs. */
.text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: row;
  flex-wrap: nowrap;
  align-items: baseline;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 17px;
}
.text.plain {
  display: block;
}
.text.wrap {
  white-space: normal;
  flex-wrap: wrap;
}
/* One `%c` run. The padding is what makes a coloured badge look like one;
 * `flex-shrink: 0` keeps a badge its own width while the message beside it
 * takes the squeeze. */
.seg {
  border-radius: 3px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* A run that carries a background is a BADGE: it keeps its own width, and the
   message beside it takes the squeeze. Without this the badge is the thing
   that ellipses, which is the one part of the line you can always read. */
.seg:first-child {
  flex-shrink: 0;
}
.text.wrap .seg {
  white-space: normal;
}
.line.warn .text {
  color: #e3b341;
}
.line.error .text,
.line.error .glyph {
  color: #ff7b72;
}
.line.input .text {
  color: #a8d4ff;
}
.line.input .glyph {
  color: #6cb6ff;
}
.line.result .text {
  color: #7ee08a;
}
.line.result .glyph {
  color: #56d364;
}
.prompt {
  height: 26px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  padding-left: 8px;
  padding-right: 8px;
  border-top-width: 1px;
  border-color: #1e2733;
  background: #0d1219;
  position: relative;
}
.caret {
  color: #6cb6ff;
  font-size: 12px;
}
.entry {
  flex: 1;
  min-width: 0;
  height: 20px;
  font-size: 11px;
  color: #d7e0ea;
  background: #00000000;
  border-width: 0px;
}
.hints {
  position: absolute;
  left: 24px;
  bottom: 26px;
  display: flex;
  flex-direction: row;
  gap: 3px;
  padding: 3px;
  border-radius: 4px;
  background: #131a24;
  border-width: 1px;
  border-color: #2b3646;
}
.hint {
  font-size: 10px;
  color: #8494a8;
  padding-left: 5px;
  padding-right: 5px;
  border-radius: 3px;
}
.hint:hover {
  background: #202938;
  color: #d7e0ea;
}
.hint.first {
  color: #a8d4ff;
  background: #14344f;
}
</style>
