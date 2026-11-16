<script setup lang="ts">
import { computed, ref } from "vue";
import type { Declaration, Rule } from "../protocol";
import { cssColor, cssish, hslaCss, nodeLabel, parseHsla, topLevel } from "../dump";
import { refreshStyles, state } from "../store";

const props = defineProps<{ height: number }>();

const node = computed(() => (state.selected === null ? null : state.byId[state.selected] || null));
const rules = computed(() => state.rules?.rules || []);
const copied = ref("");

const winners = computed(() => {
  const best: Record<string, { rule: number; decl: number; important: boolean }> = {};
  rules.value.forEach((rule, ri) => {
    rule.declarations.forEach((decl, di) => {
      const held = best[decl.property];
      const important = !!decl.important;
      if (!held || important || !held.important) {
        best[decl.property] = { rule: ri, decl: di, important };
      }
    });
  });
  return best;
});

function overridden(ruleIndex: number, declIndex: number, decl: Declaration): boolean {
  const won = winners.value[decl.property];
  return !won || won.rule !== ruleIndex || won.decl !== declIndex;
}

const ordered = computed(() =>
  rules.value
    .map((rule, i) => ({ rule, index: i }))
    .slice()
    .reverse(),
);

function stateBadge(rule: Rule): string {
  if (rule.ancestorState) return `group-${rule.state}`;
  return rule.state === "base" ? "" : rule.state;
}

const buckets = computed(() => {
  const all = state.styles?.computed || {};
  const names = Object.keys(all);
  const order = ["live", "base", "hover", "active", "focus", "focus-visible", "focus-within"];
  return names.sort((a, b) => {
    const ai = order.indexOf(a);
    const bi = order.indexOf(b);
    return (ai < 0 ? 99 : ai) - (bi < 0 ? 99 : bi) || a.localeCompare(b);
  });
});

const openBuckets = ref<Record<string, boolean>>({ live: true });

function bucketRows(name: string): { key: string; value: string; swatch: string | null }[] {
  const lines = state.styles?.computed?.[name];
  return topLevel(lines).map((entry) => {
    const hsla = parseHsla(entry.value);
    return {
      key: cssish(entry.key),
      value: entry.value,
      swatch: hsla ? hslaCss(hsla) : null,
    };
  });
}

async function copy(text: string, tag: string): Promise<void> {
  try {
    await globalThis.navigator?.clipboard?.writeText(text);
    copied.value = tag;
    setTimeout(() => {
      if (copied.value === tag) copied.value = "";
    }, 1200);
  } catch {
    copied.value = "";
  }
}

function ruleText(rule: Rule): string {
  const body = rule.declarations
    .map((d) => `  ${d.property}: ${d.value}${d.important ? " !important" : ""};`)
    .join("\n");
  const open = rule.media ? `@media ${rule.media} {\n` : "";
  const close = rule.media ? "\n}" : "";
  return `${open}${rule.selector} {\n${body}\n}${close}`;
}
</script>

<template>
  <section class="pane styles" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Styles</span>
      <span v-if="node" class="who mono">{{ nodeLabel(node) }}</span>
      <span v-if="state.styles && state.styles.hovered" class="chip warn">:hover live</span>
      <div class="spacer"></div>
      <div class="stabs">
        <div class="stab" :class="{ on: state.stylesTab === 'rules' }" @click="state.stylesTab = 'rules'">
          rules<span v-if="rules.length" class="n">{{ rules.length }}</span>
        </div>
        <div class="stab" :class="{ on: state.stylesTab === 'computed' }" @click="state.stylesTab = 'computed'">
          computed
        </div>
      </div>
      <div class="btn" title="Re-read this node's styles" @click="refreshStyles()">↻</div>
    </header>

    <div v-if="state.selected === null" class="empty">
      Select an element on the left, or press <b>Ctrl+Shift+C</b> and click it in the app.
    </div>

    <div v-else class="body">
      <template v-if="state.stylesTab === 'rules'">
        <div v-if="state.rules && state.rules.note" class="note">{{ state.rules.note }}</div>
        <div v-if="!rules.length" class="empty">
          No rule in any sheet matched this node. Its box comes from the element defaults and whatever it inherits.
        </div>
        <div v-for="entry in ordered" :key="entry.index" class="rule">
          <div class="rule-head">
            <span class="selector mono">{{ entry.rule.selector }}</span>
            <div class="spacer"></div>
            <span v-if="stateBadge(entry.rule)" class="badge state">{{ stateBadge(entry.rule) }}</span>
            <span v-if="entry.rule.pseudo" class="badge pseudo">::{{ entry.rule.pseudo }}</span>
            <span class="badge spec" :title="`specificity ${entry.rule.specificity}, source order ${entry.rule.order}`"
              >{{ entry.rule.specificity }}·{{ entry.rule.order }}</span
            >
            <span class="btn copy" :title="'Copy this rule'" @click="copy(ruleText(entry.rule), 'r' + entry.index)">{{
              copied === "r" + entry.index ? "✓" : "⧉"
            }}</span>
          </div>
          <div v-if="entry.rule.media" class="media mono">@media {{ entry.rule.media }}</div>
          <div class="decls">
            <div
              v-for="(decl, di) in entry.rule.declarations"
              :key="decl.property + di"
              class="decl mono"
              :class="{ dead: overridden(entry.index, di, decl) }"
            >
              <span
                v-if="cssColor(decl.value)"
                class="swatch"
                :style="{ background: cssColor(decl.value) || 'transparent' }"
              ></span>
              <span class="prop">{{ decl.property }}</span>
              <span class="colon">:</span>
              <span class="val">{{ decl.value }}</span>
              <span v-if="decl.important" class="bang">!important</span>
            </div>
          </div>
          <div v-if="entry.rule.diagnostics && entry.rule.diagnostics.length" class="advisories">
            <div v-for="(d, i) in entry.rule.diagnostics" :key="i" class="advisory">
              <span class="badge advise">advisory</span><span class="text">{{ d }}</span>
            </div>
          </div>
        </div>
      </template>

      <template v-else>
        <div v-if="state.stylesError" class="empty t-red">{{ state.stylesError }}</div>
        <div v-for="name in buckets" :key="name" class="bucket">
          <div class="bucket-head" @click="openBuckets[name] = !openBuckets[name]">
            <span class="twisty">{{ openBuckets[name] ? "▾" : "▸" }}</span>
            <span class="bucket-name mono">{{ name }}</span>
            <span v-if="name === 'live'" class="badge live">in effect this frame</span>
            <span class="count">{{ bucketRows(name).length }}</span>
          </div>
          <div v-if="openBuckets[name]" class="decls">
            <div v-for="row in bucketRows(name)" :key="name + row.key" class="decl mono">
              <span v-if="row.swatch" class="swatch" :style="{ background: row.swatch }"></span>
              <span class="prop">{{ row.key }}</span>
              <span class="colon">:</span>
              <span class="val">{{ row.value }}</span>
              <span class="btn copy" @click="copy(row.key + ': ' + row.value, name + row.key)">{{
                copied === name + row.key ? "✓" : "⧉"
              }}</span>
            </div>
          </div>
        </div>
      </template>
    </div>
  </section>
</template>

<style scoped>
.styles {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.who {
  font-size: var(--fs-s);
  color: var(--text);
}
.stabs {
  display: flex;
  flex-direction: row;
  gap: var(--sp-1);
}
.stab {
  height: var(--h-chip);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  border-radius: var(--r-m);
  font-size: var(--fs-xs);
  color: var(--text-muted);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
}
.stab:hover {
  color: var(--text);
  background: var(--surface-raised);
}
.stab.on {
  background: var(--surface-active);
  color: var(--text);
}
.n {
  font-size: var(--fs-2xs);
  color: var(--text-faint);
}
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: var(--sp-4);
}
.note {
  margin: var(--sp-4);
  padding: var(--sp-3);
  border-radius: var(--r-m);
  background: var(--warn-soft);
  color: var(--warn);
  font-size: var(--fs-xs);
}
.rule {
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
  padding-top: var(--sp-2);
  padding-bottom: var(--sp-2);
}
.rule-head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  padding-left: var(--sp-4);
  padding-right: var(--sp-3);
}
.selector {
  font-size: var(--fs-s);
  color: var(--warn);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge {
  height: 15px;
  padding-left: var(--sp-2);
  padding-right: var(--sp-2);
  border-radius: var(--r-s);
  font-size: var(--fs-2xs);
  display: flex;
  align-items: center;
  background: var(--surface-hover);
  color: var(--text-muted);
  white-space: nowrap;
}
.badge.state {
  background: var(--surface-hover);
  color: var(--cyan);
}
.badge.pseudo {
  background: var(--info-soft);
  color: var(--info);
}
.badge.spec {
  color: var(--text-faint);
}
.badge.live {
  background: var(--ok-soft);
  color: var(--ok);
}
.badge.advise {
  background: var(--warn-soft);
  color: var(--warn);
}
.copy {
  height: 15px;
  padding-left: var(--sp-2);
  padding-right: var(--sp-2);
  font-size: var(--fs-xs);
  border-color: transparent;
  background: transparent;
  color: var(--text-faint);
}
.copy:hover {
  color: var(--text);
  background: var(--surface-hover);
}
.media {
  padding-left: var(--sp-5);
  font-size: var(--fs-xs);
  color: var(--cyan);
}
.decls {
  padding-left: var(--sp-6);
  padding-right: var(--sp-4);
  padding-top: var(--sp-1);
}
.decl {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-1);
  font-size: var(--fs-s);
  height: 17px;
  white-space: nowrap;
}
.decl:hover {
  background: var(--surface-hover);
}
.decl.dead .prop,
.decl.dead .val {
  color: var(--text-ghost);
  text-decoration: line-through;
}
.prop {
  color: var(--text-muted);
}
.colon {
  color: var(--text-ghost);
}
.val {
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
}
.bang {
  color: var(--bad);
  font-size: var(--fs-xs);
}
.swatch {
  width: 9px;
  height: 9px;
  border-radius: var(--r-s);
  border-width: 1px;
  border-color: var(--line-strong);
  flex-shrink: 0;
}
.advisories {
  padding-left: var(--sp-6);
  padding-right: var(--sp-4);
  padding-top: var(--sp-1);
}
.advisory {
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: var(--sp-2);
  padding-top: var(--sp-1);
}
.advisory .text {
  font-size: var(--fs-xs);
  color: var(--warn);
  line-height: 14px;
}
.bucket {
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
}
.bucket-head {
  height: var(--h-head);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  background: var(--surface-panel);
}
.bucket-head:hover {
  background: var(--surface-hover);
}
.twisty {
  color: var(--text-faint);
  font-size: var(--fs-2xs);
  width: 10px;
}
.bucket-name {
  font-size: var(--fs-s);
  color: var(--text);
}
.count {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
</style>
