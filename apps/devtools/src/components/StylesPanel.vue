<script setup lang="ts">
// Styles: which rules matched, which declarations survived, and what the
// cascade produced.
//
// ## Winner first
//
// `rules` answers in CASCADE order — ascending `(specificity, order)`, so the
// LAST declaration of a property is the one in effect (docs/DEVTOOLS.md, "Rule
// provenance"). That is the right order for a machine and the wrong one for a
// person: you look at a styles panel to find out what won, and browsers have
// trained everyone to read the winner at the top. So the list is reversed here
// and the arithmetic is done once, in `winners()`.
//
// ## What is struck through
//
// A declaration is overridden when a LATER rule (or an `!important` one)
// declares the same property. That is the cascade's own rule and it is
// re-implemented here on the host's own output rather than guessed at: same
// input, same sort key, same answer — which is why a strike cannot lie about a
// winner (docs/DEVTOOLS-UI.md §4).
//
// ## The badges
//
// Every rule carries facts a browser's styles panel has no column for, because
// no browser has them: which STATE bucket the rule folds into (`hover`,
// `focus-visible`, `group-hover`), whether that state belongs to an ANCESTOR,
// and the advisories vn-css produced for that rule's own declarations. The last
// one is the answer to "why is this property doing nothing", shown next to the
// property instead of in a log the developer has to go find.
import { computed, ref } from "vue";
import type { Declaration, Rule } from "../protocol";
import { cssColor, cssish, hslaCss, nodeLabel, parseHsla, topLevel } from "../dump";
import { refreshStyles, state } from "../store";

const props = defineProps<{ height: number }>();

const node = computed(() => (state.selected === null ? null : state.byId[state.selected] || null));
const rules = computed(() => state.rules?.rules || []);
const copied = ref("");

/** `property -> the index of the rule+declaration that WON it`. */
const winners = computed(() => {
  const best: Record<string, { rule: number; decl: number; important: boolean }> = {};
  rules.value.forEach((rule, ri) => {
    rule.declarations.forEach((decl, di) => {
      const held = best[decl.property];
      const important = !!decl.important;
      // `!important` beats everything that is not; among equals the later one
      // wins, and the list is ascending, so "later" is simply "seen after".
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

/** Winner first — the list as a person reads it. */
const ordered = computed(() =>
  rules.value.map((rule, i) => ({ rule, index: i })).slice().reverse(),
);

/** The state bucket, as a badge. `base` is the common case and gets none. */
function stateBadge(rule: Rule): string {
  if (rule.ancestorState) return `group-${rule.state}`;
  return rule.state === "base" ? "" : rule.state;
}

/** The computed buckets, `live` first — the style in EFFECT this frame. */
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
    // `navigator` is declared optional in engine.d.ts (an embedder can
    // build redjs without it) and `clipboard` needs vn-app's `clipboard`
    // feature, so both are probed rather than assumed.
    await globalThis.navigator?.clipboard?.writeText(text);
    copied.value = tag;
    setTimeout(() => {
      if (copied.value === tag) copied.value = "";
    }, 1200);
  } catch {
    copied.value = "";
  }
}

/** The whole rule, as CSS, for the clipboard. */
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
        <div
          class="stab"
          :class="{ on: state.stylesTab === 'rules' }"
          @click="state.stylesTab = 'rules'"
        >
          rules<span v-if="rules.length" class="n">{{ rules.length }}</span>
        </div>
        <div
          class="stab"
          :class="{ on: state.stylesTab === 'computed' }"
          @click="state.stylesTab = 'computed'"
        >
          computed
        </div>
      </div>
      <div class="btn" title="Re-read this node's styles" @click="refreshStyles()">↻</div>
    </header>

    <div v-if="state.selected === null" class="empty">
      Select an element on the left, or press <b>Ctrl+Shift+C</b> and hover it in the app.<br />
      <span class="t-faint"
        >The picker latches when the pointer LEAVES the app's window — hover what you want, then
        come back here.</span
      >
    </div>

    <div v-else class="body">
      <!-- ---- rules ---------------------------------------------------- -->
      <template v-if="state.stylesTab === 'rules'">
        <div v-if="state.rules && state.rules.note" class="note">{{ state.rules.note }}</div>
        <div v-if="!rules.length" class="empty">
          No rule in any sheet matched this node. Its box comes from the element defaults and
          whatever it inherits.
        </div>
        <div v-for="entry in ordered" :key="entry.index" class="rule">
          <div class="rule-head">
            <span class="selector mono">{{ entry.rule.selector }}</span>
            <div class="spacer"></div>
            <span v-if="stateBadge(entry.rule)" class="badge state">{{
              stateBadge(entry.rule)
            }}</span>
            <span v-if="entry.rule.pseudo" class="badge pseudo">::{{ entry.rule.pseudo }}</span>
            <span class="badge spec" :title="`specificity ${entry.rule.specificity}, source order ${entry.rule.order}`"
              >{{ entry.rule.specificity }}·{{ entry.rule.order }}</span
            >
            <span
              class="btn copy"
              :title="'Copy this rule'"
              @click="copy(ruleText(entry.rule), 'r' + entry.index)"
              >{{ copied === "r" + entry.index ? "✓" : "⧉" }}</span
            >
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

      <!-- ---- computed ------------------------------------------------- -->
      <template v-else>
        <div v-if="state.stylesError" class="empty t-red">{{ state.stylesError }}</div>
        <div v-for="name in buckets" :key="name" class="bucket">
          <div
            class="bucket-head"
            @click="openBuckets[name] = !openBuckets[name]"
          >
            <span class="twisty">{{ openBuckets[name] ? "▾" : "▸" }}</span>
            <span class="bucket-name mono">{{ name }}</span>
            <span v-if="name === 'live'" class="badge live">in effect this frame</span>
            <span class="count">{{ bucketRows(name).length }}</span>
          </div>
          <div v-if="openBuckets[name]" class="decls">
            <div v-for="row in bucketRows(name)" :key="name + row.key" class="decl mono">
              <span
                v-if="row.swatch"
                class="swatch"
                :style="{ background: row.swatch }"
              ></span>
              <span class="prop">{{ row.key }}</span>
              <span class="colon">:</span>
              <span class="val">{{ row.value }}</span>
              <span
                class="btn copy"
                @click="copy(row.key + ': ' + row.value, name + row.key)"
                >{{ copied === name + row.key ? "✓" : "⧉" }}</span
              >
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
  font-size: 11px;
  color: #79c0ff;
}
/* `.stab`, not `.tab`: the toolbar's panel tabs are `.tab`, and two
 * different meanings behind one class name is how a selector in a spec (or in
 * somebody's head) quietly matches the wrong thing. */
.stabs {
  display: flex;
  flex-direction: row;
  gap: 2px;
}
.stab {
  height: 18px;
  padding-left: 8px;
  padding-right: 8px;
  border-radius: 4px;
  font-size: 10px;
  color: #8494a8;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
}
.stab:hover {
  color: #d7e0ea;
  background: #171e29;
}
.stab.on {
  background: #14344f;
  color: #a8d4ff;
}
.n {
  font-size: 9px;
  color: #6d7c90;
}
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 10px;
}
.note {
  margin: 8px;
  padding: 6px;
  border-radius: 4px;
  background: #221b0c;
  color: #e3b341;
  font-size: 10px;
}
.rule {
  border-bottom-width: 1px;
  border-color: #161d27;
  padding-top: 5px;
  padding-bottom: 5px;
}
.rule-head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 5px;
  padding-left: 8px;
  padding-right: 6px;
}
.selector {
  font-size: 11px;
  color: #e6c07b;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge {
  height: 15px;
  padding-left: 5px;
  padding-right: 5px;
  border-radius: 3px;
  font-size: 9px;
  display: flex;
  align-items: center;
  background: #1a2230;
  color: #8494a8;
  white-space: nowrap;
}
.badge.state {
  background: #16303f;
  color: #76e3ea;
}
.badge.pseudo {
  background: #241a33;
  color: #d2a8ff;
}
.badge.spec {
  color: #5c6b80;
}
.badge.live {
  background: #0f2318;
  color: #7ee08a;
}
.badge.advise {
  background: #2a1f07;
  color: #e3b341;
}
.copy {
  height: 15px;
  padding-left: 4px;
  padding-right: 4px;
  font-size: 10px;
  border-color: #00000000;
  background: #00000000;
  color: #5c6b80;
}
.copy:hover {
  color: #d7e0ea;
  background: #202938;
}
.media {
  padding-left: 14px;
  font-size: 10px;
  color: #76e3ea;
}
.decls {
  padding-left: 18px;
  padding-right: 8px;
  padding-top: 2px;
}
.decl {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 3px;
  font-size: 11px;
  height: 17px;
  white-space: nowrap;
}
.decl:hover {
  background: #ffffff08;
}
.decl.dead .prop,
.decl.dead .val {
  color: #4e5b6d;
  text-decoration: line-through;
}
.prop {
  color: #9db3c9;
}
.colon {
  color: #46556a;
}
.val {
  color: #a5d6ff;
  overflow: hidden;
  text-overflow: ellipsis;
}
.bang {
  color: #ff7b72;
  font-size: 10px;
}
.swatch {
  width: 9px;
  height: 9px;
  border-radius: 2px;
  border-width: 1px;
  border-color: #ffffff33;
  flex-shrink: 0;
}
.advisories {
  padding-left: 18px;
  padding-right: 8px;
  padding-top: 3px;
}
.advisory {
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: 5px;
  padding-top: 2px;
}
.advisory .text {
  font-size: 10px;
  color: #c6a663;
  line-height: 14px;
}
.bucket {
  border-bottom-width: 1px;
  border-color: #161d27;
}
.bucket-head {
  height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 5px;
  padding-left: 8px;
  padding-right: 8px;
  background: #131a24;
}
.bucket-head:hover {
  background: #182130;
}
.twisty {
  color: #6d7c90;
  font-size: 9px;
  width: 10px;
}
.bucket-name {
  font-size: 11px;
  color: #d7e0ea;
}
.count {
  margin-left: auto;
  font-size: 10px;
  color: #5c6b80;
}
</style>
