<script setup lang="ts">
// It stopped fitting the moment a target could have several WINDOWS: "which of these five
// processes, and which of its three windows" is a table, not a pill row.
import { computed } from "vue";
import type { Target } from "../protocol";
import { attach, bestTarget, capture, detach, refreshTargets, refreshWindows, selectWindow, state } from "../store";

const props = defineProps<{ height: number }>();

const activeWindow = computed(() =>
  state.window === null ? (state.windows.find((w) => w.kind === "main")?.id ?? null) : state.window,
);

async function rescan(): Promise<void> {
  const found = await refreshTargets();
  if (!state.target || state.status === "lost") {
    const next = bestTarget(found);
    if (next) void attach(next);
  }
}

function isCurrent(t: Target): boolean {
  return !!state.target && state.target.dir === t.dir;
}

function inspectorFor(id: number | null): number | null {
  const found = state.inspectors.find((i) => i.window === id);
  return found ? found.pid : null;
}

function pickWindow(id: number): void {
  const w = state.windows.find((x) => x.id === id);
  void selectWindow(w && w.kind === "main" ? null : id);
}
</script>

<template>
  <section class="pane targets" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Targets</span>
      <span class="chip">{{ state.targets.length }} instances</span>
      <div class="spacer"></div>
      <div class="btn" @click="rescan()">rescan</div>
      <div v-if="state.target" class="btn" @click="detach()">detach</div>
    </header>

    <div class="body">
      <div v-if="state.error" class="empty t-red">{{ state.error }}</div>

      <div class="section">
        <div class="head">instances</div>
        <div class="cols mono">
          <span class="c-slot">slot</span>
          <span class="c-pid">pid</span>
          <span class="c-port">port</span>
          <span class="c-dir">dist</span>
          <span class="c-state">state</span>
        </div>
        <div
          v-for="t in state.targets"
          :key="t.dir"
          class="row mono"
          :class="{ on: isCurrent(t), dead: !t.alive }"
          @click="attach(t)"
        >
          <span class="c-slot">{{ t.slot }}</span>
          <span class="c-pid">{{ t.pid }}</span>
          <span class="c-port">{{ t.port }}</span>
          <span class="c-dir" :title="t.dir">{{ t.dir }}</span>
          <span class="c-state">
            <span v-if="t.self" class="badge self">this window</span>
            <span v-else-if="!t.alive" class="badge dead">stale handshake</span>
            <span v-else-if="isCurrent(t)" class="badge on">attached</span>
            <span v-else class="badge">live</span>
          </span>
        </div>
        <div v-if="!state.targets.length" class="empty">
          No devtools handshake found. A host publishes one into its dist slot at boot (<b class="mono"
            >&lt;slot&gt;/devtools.json</b
          >) — start an app, and this list picks it up within a few seconds.
        </div>
      </div>

      <div class="section">
        <div class="head">
          windows of the attached instance
          <span class="btn" @click="refreshWindows()">↻</span>
        </div>
        <div class="cols mono">
          <span class="c-slot">window</span>
          <span class="c-pid">root node</span>
          <span class="c-port">size</span>
          <span class="c-dir">title</span>
          <span class="c-state">state</span>
        </div>
        <div
          v-for="w in state.windows"
          :key="w.id"
          class="row mono"
          :class="{ on: activeWindow === w.id }"
          @click="pickWindow(w.id)"
        >
          <span class="c-slot">{{ w.kind }} · {{ w.id }}</span>
          <span class="c-pid">{{ w.root || "document" }}</span>
          <span class="c-port">{{ w.viewport.w }}×{{ w.viewport.h }}</span>
          <span class="c-dir" :title="w.title">{{ w.title }}</span>
          <span class="c-state">
            <span v-if="w.focused" class="badge focus">focused</span>
            <span v-if="w.hovered" class="badge">pointer</span>
            <span v-if="inspectorFor(w.id)" class="badge insp">inspector pid {{ inspectorFor(w.id) }}</span>
          </span>
        </div>
        <div v-if="!state.windows.length" class="empty">Not attached.</div>
      </div>

      <div class="section">
        <div class="head">
          preview
          <span class="btn" :class="{ on: state.shotBusy }" @click="capture()">{{
            state.shotBusy ? "capturing…" : "capture"
          }}</span>
          <span v-if="state.shot" class="t-faint"
            >{{ state.shot.width }}×{{ state.shot.height }} · {{ state.shot.backend }}</span
          >
        </div>
        <div v-if="state.shotError" class="empty t-red">{{ state.shotError }}</div>
        <div v-else-if="state.shot" class="shot">
          <img :src="state.shot.url" class="img" />
          <div v-if="state.shot.note" class="note">{{ state.shot.note }}</div>
        </div>
        <div v-else class="empty">
          A capture of the selected window's client area — unfocused, occluded, never raised. On demand only: a live
          preview would cost the target more than every other command here put together.
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.targets {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: var(--sp-6);
}
.section {
  padding-top: var(--sp-4);
}
.head {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-5);
  padding-bottom: var(--sp-2);
  font-size: var(--fs-xs);
  color: var(--text-muted);
  letter-spacing: 0.4px;
  text-transform: uppercase;
}
.cols {
  height: var(--h-head);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-5);
  font-size: var(--fs-xs);
  color: var(--text-faint);
  background: var(--surface-panel);
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
}
.row {
  height: var(--h-row-loose);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-5);
  font-size: var(--fs-s);
  color: var(--text);
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
  white-space: nowrap;
}
.row:hover {
  background: var(--surface-hover);
}
.row.on {
  background: var(--surface-active);
}
.row.dead {
  color: var(--text-ghost);
}
.c-slot {
  width: 150px;
  flex-shrink: 0;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
}
.c-pid,
.c-port {
  width: 80px;
  flex-shrink: 0;
  color: var(--text-muted);
}
.c-dir {
  flex: 1;
  min-width: 0;
  color: var(--text-faint);
  overflow: hidden;
  text-overflow: ellipsis;
}
.c-state {
  width: 190px;
  flex-shrink: 0;
  display: flex;
  flex-direction: row;
  gap: var(--sp-2);
  justify-content: flex-end;
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
}
.badge.on {
  background: var(--surface-active);
  color: var(--text);
}
.badge.self {
  background: var(--info-soft);
  color: var(--info);
}
.badge.dead {
  background: var(--bad-soft);
  color: var(--bad);
}
.badge.focus {
  background: var(--ok-soft);
  color: var(--ok);
}
.badge.insp {
  background: var(--surface-hover);
  color: var(--cyan);
}
.shot {
  padding: var(--sp-4);
}
.img {
  max-width: 100%;
  border-radius: var(--r-m);
  border-width: 1px;
  border-color: var(--line);
}
.note {
  padding-top: var(--sp-3);
  font-size: var(--fs-xs);
  color: var(--text-faint);
  line-height: 15px;
}
</style>
