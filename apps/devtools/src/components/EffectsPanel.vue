<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { InspectorRequest } from "../inspector-model";
import {
  controlsFor,
  cssText,
  draftFrom,
  fmtNumber,
  hexColor,
  parseShading,
  readEffects,
  type Control,
  type Draft,
  type EffectInfo,
  type EffectParam,
  type EffectsReply,
  type SetReply,
  type ShadingNode,
  urlOf,
} from "../effects-model";
import { select, state, ui } from "../store";

const props = defineProps<{
  height: number;
  request?: InspectorRequest | undefined;
  sessionKey?: string | number | undefined;
}>();

const reply = ref<EffectsReply | null>(null);
const error = ref("");
const node = ref<ShadingNode | null>(null);
const picked = ref<string | null>(null);
const draft = ref<Draft>({ values: {}, enabled: {}, urls: {} });
const applied = ref<SetReply | null>(null);
const copied = ref(false);

const effects = computed(() => (reply.value ? readEffects(reply.value.manifests, reply.value.loaded) : []));
const users = computed(() => reply.value?.nodes ?? []);

const editing = computed<EffectInfo | null>(() => {
  const name = picked.value ?? node.value?.effect ?? parseShading(node.value?.value)?.name ?? null;
  if (!name) return null;
  return effects.value.find((e) => e.name.toLowerCase() === name.toLowerCase()) ?? null;
});
const controls = computed<Control[]>(() => (editing.value ? controlsFor(editing.value) : []));
const css = computed(() => (editing.value ? cssText(editing.value, draft.value) : ""));

function label(n: { tag: string | null; classes: string[] }): string {
  return `${n.tag ?? "?"}${n.classes.map((c) => `.${c}`).join("")}`;
}

async function call<T>(payload: Record<string, unknown>): Promise<T | null> {
  if (!props.request) return null;
  try {
    const out = await props.request<T>({ cmd: "effects", ...payload });
    error.value = "";
    return out;
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
    return null;
  }
}

async function refresh(): Promise<void> {
  const list = await call<EffectsReply>({ op: "list" });
  if (list) reply.value = list;
}

async function loadNode(rebuild: boolean): Promise<void> {
  const id = state.selected;
  if (id === null) {
    node.value = null;
    if (rebuild) rebuildDraft();
    return;
  }
  const detail = await call<ShadingNode>({ op: "node", node: id });
  if (state.selected !== id) return;
  node.value = detail;
  if (rebuild) rebuildDraft();
}

function rebuildDraft(): void {
  const effect = editing.value;
  if (!effect) {
    draft.value = { values: {}, enabled: {}, urls: {} };
    return;
  }
  const n = node.value;
  const same = n?.effect?.toLowerCase() === effect.name.toLowerCase();
  draft.value = draftFrom(effect, n?.value ?? null, same ? (n?.params ?? null) : null);
}

let sending = false;
let queued = false;

async function apply(): Promise<void> {
  const id = state.selected;
  if (id === null || !editing.value) return;
  if (sending) {
    queued = true;
    return;
  }
  sending = true;
  try {
    do {
      queued = false;
      const out = await call<SetReply>({ op: "set", node: id, value: css.value });
      if (out) applied.value = out;
    } while (queued && state.selected === id);
  } finally {
    sending = false;
  }
  await loadNode(false);
  void refresh();
}

async function reset(): Promise<void> {
  const id = state.selected;
  if (id === null) return;
  await call({ op: "reset", node: id });
  applied.value = null;
  picked.value = null;
  await loadNode(true);
  void refresh();
}

function pick(effect: EffectInfo): void {
  picked.value = effect.name;
  applied.value = null;
  rebuildDraft();
}

function inputValue(event: Event): string {
  const target: unknown = event.target;
  if (target && typeof target === "object" && "value" in target) return String(target.value);
  return "";
}

function setScalar(param: EffectParam, raw: string): void {
  const v = Number(raw);
  if (!Number.isFinite(v)) return;
  draft.value.values[param.name] = [v];
  draft.value.enabled[param.name] = true;
  void apply();
}

function commitNumber(param: EffectParam, event: Event): void {
  if (event.type === "blur") ui.typing = false;
  const raw = inputValue(event).trim();
  if (raw === "" || Number(raw) === scalar(param)) return;
  setScalar(param, raw);
}

function setColor(param: EffectParam, hex: string): void {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return;
  const prev = draft.value.values[param.name] ?? [];
  const rgb = [0, 2, 4].map((i) => parseInt(m[1]!.slice(i, i + 2), 16) / 255);
  draft.value.values[param.name] = [...rgb, prev[3] ?? 1];
  draft.value.enabled[param.name] = true;
  void apply();
}

function commitUrl(param: EffectParam, event: Event): void {
  if (event.type === "blur") ui.typing = false;
  const typed = inputValue(event).trim();
  const url = urlOf(typed) ?? typed;
  if (url === (draft.value.urls[param.name] ?? "")) return;
  draft.value.urls[param.name] = url;
  draft.value.enabled[param.name] = url !== "" || param.required;
  void apply();
}

function setAlpha(param: EffectParam, raw: string): void {
  const a = Number(raw);
  if (!Number.isFinite(a)) return;
  const prev = draft.value.values[param.name] ?? [1, 1, 1, 1];
  draft.value.values[param.name] = [prev[0] ?? 1, prev[1] ?? 1, prev[2] ?? 1, a];
  draft.value.enabled[param.name] = true;
  void apply();
}

function paramsOf(control: Control): EffectParam[] {
  return control.type === "vector" ? control.parts.map((p) => p.param) : [control.param];
}

function optional(control: Control): boolean {
  return paramsOf(control).some((p) => !p.required && p.editable);
}

function isOn(control: Control): boolean {
  return paramsOf(control).every((p) => p.required || draft.value.enabled[p.name] === true);
}

function toggle(control: Control): void {
  const next = !isOn(control);
  for (const p of paramsOf(control)) if (!p.required) draft.value.enabled[p.name] = next;
  void apply();
}

function scalar(param: EffectParam): number {
  return draft.value.values[param.name]?.[0] ?? param.default[0] ?? 0;
}

function color(param: EffectParam): number[] {
  return draft.value.values[param.name] ?? param.default;
}

async function copy(): Promise<void> {
  try {
    await globalThis.navigator?.clipboard?.writeText(`--shading: ${css.value};`);
    copied.value = true;
    setTimeout(() => (copied.value = false), 1200);
  } catch {
    copied.value = false;
  }
}

watch(
  () => state.selected,
  () => {
    picked.value = null;
    applied.value = null;
    void loadNode(true);
  },
);
watch(
  () => props.sessionKey,
  () => {
    reply.value = null;
    node.value = null;
    void refresh().then(() => loadNode(true));
  },
);
watch(effects, () => {
  if (editing.value && !Object.keys(draft.value.values).length) rebuildDraft();
});
const TREE_REFRESH_MS = 1000;
let treeTimer: ReturnType<typeof setTimeout> | null = null;
watch(
  () => state.treeRev,
  () => {
    if (treeTimer !== null) return;
    treeTimer = setTimeout(() => {
      treeTimer = null;
      void refresh();
    }, TREE_REFRESH_MS);
  },
);
onUnmounted(() => {
  if (treeTimer !== null) clearTimeout(treeTimer);
});

onMounted(async () => {
  await refresh();
  await loadNode(true);
});
</script>

<template>
  <section class="pane effects" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Effects</span>
      <span class="chip">{{ effects.length }} available</span>
      <span class="chip">{{ users.length }} in use</span>
      <div class="spacer"></div>
      <button class="btn" aria-label="Refresh effects" @click="refresh()">Refresh</button>
    </header>
    <div v-if="error" class="empty t-red">{{ error }}</div>

    <div class="cols">
      <div class="side">
        <div class="section label">In use</div>
        <div v-if="!users.length" class="empty">No element paints a <span class="mono">--shading</span> effect.</div>
        <div v-for="u in users" :key="u.id" class="user" :class="{ on: state.selected === u.id }" @click="select(u.id)">
          <div class="line">
            <span class="who mono">{{ label(u) }}</span>
            <span v-if="u.overridden" class="badge edit">edited</span>
            <span v-if="u.animated" class="badge anim">animated</span>
            <span class="id">{{ u.id }}</span>
          </div>
          <div class="val mono">{{ u.value ?? "(no declaration)" }}</div>
        </div>

        <div class="section label">Available</div>
        <div
          v-for="e in effects"
          :key="(e.builtin ? 'b:' : 'a:') + e.name"
          class="fx"
          :class="{ on: editing?.name === e.name, off: !e.loaded }"
          :title="e.loaded ? e.origin : `${e.origin}: the host did not load this effect`"
          @click="pick(e)"
        >
          <div class="line">
            <span class="name mono">{{ e.name }}</span>
            <span class="badge" :class="e.builtin ? 'builtin' : 'app'">{{ e.builtin ? "built-in" : "app" }}</span>
            <span v-for="f in e.flags" :key="f.key" class="badge flag" :title="f.title">{{ f.label }}</span>
          </div>
          <div v-if="e.doc" class="doc">{{ e.doc }}</div>
          <div class="sig mono">{{ e.signature }}</div>
        </div>
      </div>

      <div class="editor">
        <div v-if="!editing" class="empty">
          <template v-if="state.selected === null"
            >Select an element that uses <span class="mono">--shading</span>, or pick an effect to see its
            parameters.</template
          >
          <template v-else>
            This element paints no effect. Pick one on the left to apply it live.
            <div v-if="node && node.value" class="mono t-red">{{ node.value }}</div>
          </template>
        </div>

        <template v-else>
          <div class="ehead">
            <span class="name mono">{{ editing.name }}</span>
            <span v-for="f in editing.flags" :key="f.key" class="badge flag" :title="f.title">{{ f.label }}</span>
            <div class="spacer"></div>
            <span v-if="state.selected !== null && node" class="target mono">{{ label(node) }}</span>
          </div>
          <div v-if="editing.doc" class="doc pad">{{ editing.doc }}</div>
          <div class="sig mono pad">{{ editing.signature }}</div>
          <div class="flagnotes pad">
            <div v-for="f in editing.flags" :key="f.key" class="flagnote">
              <span class="badge flag">{{ f.label }}</span
              ><span>{{ f.title }}</span>
            </div>
          </div>

          <div class="controls">
            <div v-for="c in controls" :key="c.key" class="ctl" :class="{ dim: !isOn(c) }">
              <button
                v-if="optional(c)"
                class="switch"
                :class="{ on: isOn(c) }"
                role="switch"
                :aria-checked="isOn(c)"
                :aria-label="`Use ${c.key}`"
                @click="toggle(c)"
              >
                <span class="knob" />
              </button>
              <span v-else class="switch-gap"></span>
              <span class="pname mono">{{ c.key }}</span>

              <template v-if="c.type === 'scalar'">
                <input
                  class="slider"
                  type="range"
                  :min="c.scalar.min"
                  :max="c.scalar.max"
                  :step="c.scalar.step"
                  :value="scalar(c.param)"
                  :aria-label="c.key"
                  @input="setScalar(c.param, inputValue($event))"
                />
                <input
                  class="input num"
                  :value="fmtNumber(scalar(c.param), c.scalar.step)"
                  @focus="ui.typing = true"
                  @blur="commitNumber(c.param, $event)"
                  @keydown.enter="commitNumber(c.param, $event)"
                />
                <span class="unit">{{ c.scalar.unit }}</span>
              </template>

              <template v-else-if="c.type === 'vector'">
                <div class="parts">
                  <div v-for="p in c.parts" :key="p.axis" class="part">
                    <span class="axis mono">{{ p.axis }}</span>
                    <input
                      class="slider"
                      type="range"
                      :min="p.scalar.min"
                      :max="p.scalar.max"
                      :step="p.scalar.step"
                      :value="scalar(p.param)"
                      :aria-label="p.param.name"
                      @input="setScalar(p.param, inputValue($event))"
                    />
                    <input
                      class="input num"
                      :value="fmtNumber(scalar(p.param), p.scalar.step)"
                      @focus="ui.typing = true"
                      @blur="commitNumber(p.param, $event)"
                      @keydown.enter="commitNumber(p.param, $event)"
                    />
                    <span class="unit">{{ p.scalar.unit }}</span>
                  </div>
                </div>
              </template>

              <template v-else-if="c.type === 'color'">
                <input
                  class="swatch"
                  type="color"
                  :value="hexColor(color(c.param), false)"
                  :aria-label="c.key"
                  @input="setColor(c.param, inputValue($event))"
                />
                <span class="hex mono">{{ hexColor(color(c.param)) }}</span>
                <span class="axis mono">α</span>
                <input
                  class="slider alpha"
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  :value="color(c.param)[3] ?? 1"
                  :aria-label="`${c.key} alpha`"
                  @input="setAlpha(c.param, inputValue($event))"
                />
              </template>

              <template v-else-if="c.type === 'texture'">
                <input
                  class="input url mono"
                  :value="draft.urls[c.param.name] ?? ''"
                  placeholder="assets/image.png"
                  :aria-label="`${c.key} image`"
                  @focus="ui.typing = true"
                  @blur="commitUrl(c.param, $event)"
                  @keydown.enter="commitUrl(c.param, $event)"
                />
                <span class="fixed" :title="c.param.linear ? 'sampled as stored ([SkLinear])' : 'a colour image'">{{
                  c.param.linear ? "data" : "colour"
                }}</span>
              </template>

              <template v-else>
                <span class="fixed">{{ c.param.kind }} · filled by the host</span>
              </template>
            </div>
          </div>

          <div class="out">
            <div class="outhead">
              <span class="label">CSS</span>
              <div class="spacer"></div>
              <button v-if="state.selected !== null" class="btn" @click="apply()">Apply</button>
              <button class="btn" :aria-label="'Copy the --shading declaration'" @click="copy()">
                {{ copied ? "Copied" : "Copy" }}
              </button>
              <button
                v-if="state.selected !== null"
                class="btn danger reset"
                :disabled="!node || !node.overridden"
                title="Drop the inspector's override and go back to the stylesheet's value"
                @click="reset()"
              >
                Reset
              </button>
            </div>
            <div class="css mono">--shading: {{ css }};</div>
            <div v-if="node && node.overridden" class="was mono">
              {{ node.authoredSource === "none" ? "was: no declaration" : `was: ${node.authored ?? ""}` }}
            </div>
            <div v-if="applied && !applied.applies" class="advisory">the host did not accept this value</div>
            <div v-for="(d, i) in applied?.diagnostics ?? []" :key="i" class="advisory">{{ d }}</div>
            <div v-if="node && node.declarations.length" class="decls">
              <div v-for="(d, i) in node.declarations" :key="i" class="decl mono">
                <span class="sel">{{ d.selector }}</span>
                <span v-if="d.state !== 'base'" class="badge">{{ d.state }}</span>
                <span class="dval">{{ d.value }}{{ d.important ? " !important" : "" }}</span>
              </div>
            </div>
          </div>
        </template>
      </div>
    </div>
  </section>
</template>

<style scoped>
.effects {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.cols {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: row;
}
.side {
  width: 340px;
  flex-shrink: 0;
  overflow-y: auto;
  border-right-width: 1px;
  border-color: var(--line);
  padding-bottom: var(--sp-5);
}
.section {
  padding-left: var(--sp-4);
  padding-top: var(--sp-4);
  padding-bottom: var(--sp-2);
}
.user,
.fx {
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  padding-top: var(--sp-2);
  padding-bottom: var(--sp-2);
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
}
.user:hover,
.fx:hover {
  background: var(--surface-hover);
}
.user.on,
.fx.on {
  background: var(--surface-active);
}
.fx.off {
  opacity: 0.5;
}
.line {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  height: 17px;
}
.who,
.name {
  font-size: var(--fs-s);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.id {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--text-ghost);
}
.val,
.sig {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.doc {
  font-size: var(--fs-xs);
  color: var(--text-faint);
  line-height: 14px;
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
.badge.builtin {
  color: var(--text-faint);
}
.badge.app {
  color: var(--cyan);
}
.badge.flag {
  background: var(--warn-soft);
  color: var(--warn);
}
.badge.edit {
  background: var(--info-soft);
  color: var(--info);
}
.badge.anim {
  background: var(--warn-soft);
  color: var(--warn);
}
.editor {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding-bottom: var(--sp-6);
}
.ehead {
  height: var(--h-bar);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  padding-left: var(--sp-5);
  padding-right: var(--sp-5);
}
.ehead .name {
  font-size: var(--fs-l);
  color: var(--text-strong);
}
.target {
  font-size: var(--fs-s);
  color: var(--text-muted);
}
.pad {
  padding-left: var(--sp-5);
  padding-right: var(--sp-5);
}
.flagnotes {
  padding-top: var(--sp-2);
}
.flagnote {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
  font-size: var(--fs-xs);
  color: var(--text-faint);
  padding-top: var(--sp-1);
}
.controls {
  padding-top: var(--sp-4);
  padding-left: var(--sp-5);
  padding-right: var(--sp-5);
}
.ctl {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
  min-height: 26px;
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
}
.ctl.dim .pname,
.ctl.dim .num,
.ctl.dim .hex {
  color: var(--text-ghost);
}
.pname {
  width: 110px;
  flex-shrink: 0;
  font-size: var(--fs-s);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.slider {
  flex: 1;
  min-width: 80px;
}
.slider.alpha {
  max-width: 160px;
}
.num {
  width: 64px;
  text-align: right;
}
.url {
  flex: 1;
  min-width: 80px;
}
.unit {
  width: 16px;
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.parts {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding-top: var(--sp-1);
  padding-bottom: var(--sp-1);
}
.part {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
}
.axis {
  width: 10px;
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.swatch {
  width: 32px;
  height: 18px;
}
.hex {
  width: 80px;
  font-size: var(--fs-s);
  color: var(--text);
}
.fixed {
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.switch {
  width: 24px;
  height: 14px;
  border-radius: 7px;
  background: var(--surface-active);
  border-width: 1px;
  border-color: var(--line-strong);
  flex-shrink: 0;
  display: flex;
  align-items: center;
  padding-left: 1px;
}
.switch.on {
  background: var(--ok-line);
  border-color: var(--ok);
  justify-content: flex-end;
  padding-right: 1px;
}
.knob {
  width: 10px;
  height: 10px;
  border-radius: 5px;
  background: var(--text-muted);
}
.switch.on .knob {
  background: var(--ok);
}
.switch-gap {
  width: 24px;
  flex-shrink: 0;
}
.out {
  margin-top: var(--sp-5);
  padding-left: var(--sp-5);
  padding-right: var(--sp-5);
}
.outhead {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  height: var(--h-bar);
}
.css {
  padding: var(--sp-3);
  border-radius: var(--r-m);
  background: var(--surface-sunken);
  border-width: 1px;
  border-color: var(--line);
  font-size: var(--fs-s);
  color: var(--text-strong);
  user-select: text;
}
.was {
  padding-top: var(--sp-2);
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.advisory {
  padding-top: var(--sp-2);
  font-size: var(--fs-xs);
  color: var(--warn);
  line-height: 14px;
}
.decls {
  padding-top: var(--sp-4);
}
.decl {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-2);
  height: 17px;
  font-size: var(--fs-xs);
  white-space: nowrap;
}
.sel {
  color: var(--warn);
  overflow: hidden;
  text-overflow: ellipsis;
}
.dval {
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
