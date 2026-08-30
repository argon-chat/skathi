<script setup>
// Two views of the same element, and the difference between them is the point.
//
//   Computed — the folded `ComputedStates` the cascade produced, bucket by
//              bucket (base / live / hover / active / focus / group-* /
//              transitions). `live` is the style in EFFECT this frame; every
//              other bucket describes the STYLESHEET. docs/DEVTOOLS.md.
//   Rules    — WHICH rules produced those values, in cascade order, with their
//              declarations as the author wrote them, the `@media` block they
//              came from, and any advisory vn-css has about them. A property
//              a later rule overrode is struck through, exactly like a browser
//              does it, and the strike is computed from the SAME order the
//              cascade folded in.
import { computed, ref } from "vue";
import { state, refreshStyles } from "../store.js";
import { topLevel, parseHsla, hslaCss, nodeLabel } from "../dump.js";

/** Declarations shown per rule before it folds. A Tailwind preflight block is
    ~45 `--tw-*` seeds, and burying the app's own rules under them is how a
    styles panel becomes unreadable. */
const FOLD_AT = 10;
const unfolded = ref({});
const showAll = (rule) => !!unfolded.value[rule.order];
const unfold = (rule) => (unfolded.value = { ...unfolded.value, [rule.order]: true });
const shownDecls = (rule) =>
  showAll(rule) ? rule.declarations : (rule.declarations || []).slice(0, FOLD_AT);
const hiddenCount = (rule) =>
  showAll(rule) ? 0 : Math.max(0, (rule.declarations || []).length - FOLD_AT);

/** Buckets in the order a reader wants them, not the order they arrive in. */
const ORDER = ["live", "base", "hover", "active", "focus", "focus-visible", "group-hover", "group-active", "transitions", "animations", "extras"];

const node = computed(() => (state.selected === null ? null : state.byId[state.selected]));

const buckets = computed(() => {
  const computedStyles = state.styles && state.styles.computed;
  if (!computedStyles) return [];
  const names = Object.keys(computedStyles);
  names.sort((a, b) => {
    const ia = ORDER.indexOf(a), ib = ORDER.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });
  return names.map((name) => ({ name, entries: topLevel(computedStyles[name]), raw: computedStyles[name] }));
});

const swatch = (value) => {
  const hsla = parseHsla(value);
  return hsla ? hslaCss(hsla) : null;
};

/**
 * Which rule owns each property, folded exactly as the cascade folds: last
 * declaration of a property in `(specificity, order)` order wins its bucket.
 * A rule's state bucket is part of the key — a `:hover` declaration does not
 * override a base one, it lives on a different rung.
 */
const winners = computed(() => {
  const map = {};
  for (const rule of (state.rules && state.rules.rules) || []) {
    const scope = `${rule.state}|${rule.pseudo || ""}|${rule.media || ""}`;
    for (const d of rule.declarations || []) {
      map[`${scope}|${d.property}`] = `${rule.order}|${d.important ? 1 : 0}`;
    }
  }
  return map;
});

const overridden = (rule, decl) => {
  const scope = `${rule.state}|${rule.pseudo || ""}|${rule.media || ""}`;
  const winner = winners.value[`${scope}|${decl.property}`];
  return winner !== `${rule.order}|${decl.important ? 1 : 0}`;
};

/**
 * WINNER FIRST. The wire hands rules over in cascade order (weakest first,
 * because that is the order they fold in), and every browser's styles panel
 * shows the opposite — the rule that decided the value is the one you came to
 * read. The strike-through still comes from the cascade order, not from this
 * one, so reversing changes what is on top and nothing about what won.
 */
const orderedRules = computed(() => {
  const rules = (state.rules && state.rules.rules) || [];
  return rules.slice().reverse();
});

const box = computed(() => (node.value && node.value.bounds) || null);
</script>

<template>
  <section class="pane">
    <header class="head">
      <span class="title">Styles</span>
      <span v-if="node" class="who">{{ nodeLabel(node) }} · #{{ node.id }}</span>
      <div class="spacer" />
      <div class="tab" :class="{ on: state.stylesTab === 'styles' }" @click="state.stylesTab = 'styles'">computed</div>
      <div class="tab" :class="{ on: state.stylesTab === 'rules' }" @click="state.stylesTab = 'rules'">
        rules<span v-if="state.rules"> ({{ state.rules.count }})</span>
      </div>
      <div class="btn" @click="refreshStyles">refresh</div>
    </header>

    <div class="body">
      <div v-if="state.selected === null" class="empty">select an element on the left, or use `pick`</div>

      <template v-else>
        <div v-if="box" class="box">
          <span class="k">box</span>
          <span class="v">{{ Math.round(box.x) }}, {{ Math.round(box.y) }} · {{ Math.round(box.w) }} × {{ Math.round(box.h) }}</span>
          <span v-if="state.styles && state.styles.hovered" class="hov">:hover is live</span>
        </div>
        <div v-else class="box">
          <span class="k">box</span>
          <span class="v muted">not painted — detached, or parked in a &lt;KeepAlive&gt; cache</span>
        </div>

        <div v-if="state.stylesError" class="error">{{ state.stylesError }}</div>

        <!-- computed ------------------------------------------------------ -->
        <template v-if="state.stylesTab === 'styles'">
          <div v-for="bucket in buckets" :key="bucket.name" class="bucket">
            <div class="bucket-name" :class="bucket.name">{{ bucket.name }}</div>
            <div v-for="entry in bucket.entries" :key="entry.key" class="decl">
              <span class="prop">{{ entry.key }}</span>
              <span class="colon">:</span>
              <span v-if="swatch(entry.value)" class="chip" :style="{ background: swatch(entry.value) }" />
              <span class="val">{{ entry.value }}</span>
            </div>
            <div v-if="!bucket.entries.length" class="decl muted">{{ bucket.raw.join(" ") }}</div>
          </div>
        </template>

        <!-- rules --------------------------------------------------------- -->
        <template v-else>
          <div v-if="state.rules && state.rules.note" class="note">{{ state.rules.note }}</div>
          <div v-for="rule in orderedRules" :key="rule.order" class="rule">
            <div class="rule-head">
              <span class="sel">{{ rule.selector }}</span>
              <span v-if="rule.pseudo" class="pseudo">{{ rule.pseudo }}</span>
              <span v-if="rule.state !== 'base'" class="state">{{ rule.ancestorState ? "group-" : "" }}{{ rule.state }}</span>
              <div class="spacer" />
              <span class="ord">spec {{ rule.specificity }} · #{{ rule.order }}</span>
            </div>
            <div v-if="rule.media" class="media">@media {{ rule.media }}</div>
            <div
              v-for="(d, i) in shownDecls(rule)"
              :key="i"
              class="decl"
              :class="{ dead: overridden(rule, d) }"
            >
              <span class="prop">{{ d.property }}</span>
              <span class="colon">:</span>
              <span v-if="swatch(d.value)" class="chip" :style="{ background: d.value }" />
              <span class="val">{{ d.value }}<span v-if="d.important" class="bang"> !important</span></span>
            </div>
            <div v-if="hiddenCount(rule)" class="fold" @click="unfold(rule)">
              +{{ hiddenCount(rule) }} more declarations
            </div>
            <div v-for="(diag, i) in rule.diagnostics || []" :key="'d' + i" class="advisory">{{ diag }}</div>
          </div>
          <div v-if="state.rules && !state.rules.count" class="empty">no rule in the sheet matches this element</div>
        </template>
      </template>
    </div>
  </section>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  /* NO `height: 100%` here: the SHELL sizes the panes (App.vue), and a
     `height` on a component root would collide with the parent's rule at the
     same specificity — whichever module the builder emitted last would win,
     which is not a layout, it is a coin flip. */
  min-height: 0;
}
.head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding-left: 10px;
  padding-right: 8px;
  height: 30px;
  background: #ffffff08;
  border-bottom-width: 1px;
  border-color: #ffffff14;
}
.title {
  font-size: 11px;
  color: #cfe1ff;
}
.who {
  font-size: 10px;
  font-family: Consolas, monospace;
  color: #ffd479;
}
.spacer {
  flex: 1;
}
.tab,
.btn {
  font-size: 10px;
  color: #ffffff99;
  padding-left: 7px;
  padding-right: 7px;
  padding-top: 2px;
  padding-bottom: 2px;
  border-radius: 4px;
  border-width: 1px;
  border-color: #ffffff26;
  cursor: pointer;
  transition: background-color 120ms ease, color 120ms ease;
}
.tab:hover,
.btn:hover {
  background: #ffffff1a;
  color: #ffffff;
}
.tab.on {
  background: #1e4c8a;
  border-color: #6cb6ff;
  color: #ffffff;
}
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 14px;
}
.empty,
.note {
  padding: 12px;
  font-size: 11px;
  color: #ffffff70;
}
.note {
  color: #ffd479;
}
.error {
  font-size: 10px;
  color: #ff7b72;
  padding: 6px;
  background: #ff7b721a;
}
.box {
  display: flex;
  flex-direction: row;
  gap: 8px;
  padding: 6px;
  font-size: 11px;
  font-family: Consolas, monospace;
  background: #ffffff08;
  border-bottom-width: 1px;
  border-color: #ffffff14;
}
.box .k {
  color: #6cb6ff;
}
.box .v {
  color: #c8d6e5;
}
.box .muted {
  color: #ffffff59;
}
.box .hov {
  color: #7ee787;
}
.bucket {
  padding-top: 6px;
}
.bucket-name {
  font-size: 10px;
  color: #ffffff99;
  background: #ffffff0f;
  padding-left: 8px;
  padding-top: 2px;
  padding-bottom: 2px;
}
.bucket-name.live {
  color: #7ee787;
}
.decl {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
  padding-left: 14px;
  font-size: 11px;
  font-family: Consolas, monospace;
  color: #c8d6e5;
}
.decl.dead {
  color: #ffffff45;
  text-decoration: line-through;
}
.decl.muted {
  color: #ffffff59;
}
.prop {
  color: #6cb6ff;
}
.colon {
  color: #ffffff59;
}
.chip {
  width: 9px;
  height: 9px;
  border-radius: 2px;
  border-width: 1px;
  border-color: #ffffff59;
}
.bang {
  color: #ff7b72;
}
.rule {
  padding-top: 6px;
  padding-bottom: 6px;
  border-bottom-width: 1px;
  border-color: #ffffff0f;
}
.rule-head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 6px;
  padding-left: 8px;
  padding-right: 8px;
}
.sel {
  font-size: 11px;
  font-family: Consolas, monospace;
  color: #ffd479;
}
.pseudo {
  font-size: 10px;
  color: #d2a8ff;
}
.state {
  font-size: 10px;
  color: #7ee787;
  background: #7ee7871a;
  padding-left: 5px;
  padding-right: 5px;
  border-radius: 4px;
}
.ord {
  font-size: 10px;
  color: #ffffff45;
}
.media {
  padding-left: 8px;
  font-size: 10px;
  color: #d2a8ff;
}
.fold {
  padding-left: 14px;
  font-size: 10px;
  color: #6cb6ff;
  cursor: pointer;
}
.fold:hover {
  color: #ffffff;
}
.advisory {
  margin-left: 8px;
  margin-right: 8px;
  margin-top: 4px;
  padding: 5px;
  font-size: 10px;
  color: #ffd479;
  background: #ffd4791a;
  border-radius: 4px;
}
</style>
