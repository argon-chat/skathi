<script setup>
// The permissions surface, both tiers — docs/PERMISSIONS.md.
//
// Every row below is read TWICE on purpose, and the page shows both answers
// side by side, because the whole design is that they are different surfaces:
//
//   tier 1  navigator.permissions.query({ name })  -> "granted"|"denied"|"prompt"
//           The W3C API, byte-identical to a browser build. Three states.
//   tier 2  appPermissions.query(name)             -> + "unsupported"
//           The vue-native extension, which also carries request(). Four states.
//
// A capability the platform has no notion of reads `unsupported` on the right
// and `denied` on the left — that column pair IS the divergence, made visible.
import { onMounted, onUnmounted, ref } from "vue";
import { appPermissions } from "vue-native";

defineOptions({ name: "PermissionsPage" });

const platform = ref(appPermissions.platform);
// { name, state (4-state), webState (3-state), canRequest, requesting }
const rows = ref([]);
const changes = ref([]);
const lastRequest = ref("");

// The standard-tier `PermissionStatus` objects, kept so the page can show the
// web-facing `state` next to the extension's. They are also the change source:
// `appPermissions` re-broadcasts from these very objects rather than opening a
// second native subscription.
const statuses = new Map();

async function readAll() {
  const list = appPermissions.list();
  for (const r of list) {
    if (!statuses.has(r.name)) {
      // A name the standard tier rejects would be a bug in one of the two
      // tables; catch it rather than leaving the row blank with no reason.
      try {
        statuses.set(r.name, await navigator.permissions.query({ name: r.name }));
      } catch (e) {
        statuses.set(r.name, { state: `!${e.message}` });
      }
    }
    r.webState = statuses.get(r.name).state;
    r.requesting = false;
  }
  rows.value = list;
}

const onChange = (e) => {
  changes.value = [`${e.name} -> ${e.state}`, ...changes.value].slice(0, 6);
  readAll();
};

onMounted(() => {
  readAll();
  appPermissions.addEventListener("change", onChange);
});
onUnmounted(() => appPermissions.removeEventListener("change", onChange));

// THE EXTENSION. There is no browser call this maps to: a browser asks for the
// microphone as a side effect of getUserMedia() and never on its own.
//
// On Windows this opens the matching ms-settings: page and the promise settles
// when the consent store actually moves — or after the backend's timeout, with
// the unchanged state, so the `await` below can never hang forever.
async function request(row) {
  row.requesting = true;
  lastRequest.value = `${row.name}: asking…`;
  try {
    const state = await appPermissions.request(row.name);
    lastRequest.value = `${row.name}: ${state}`;
  } catch (e) {
    lastRequest.value = `${row.name}: ${e.message}`;
  } finally {
    row.requesting = false;
    readAll();
  }
}

const cls = (state) =>
  state === "granted" ? "ok" : state === "denied" ? "no" : state === "prompt" ? "ask" : "unk";
</script>

<template>
  <div class="page">
    <div class="hd">
      <span class="h1">Permissions</span>
      <span class="sub">docs/PERMISSIONS.md — two tiers, one platform trait</span>
      <span class="pill live" id="perm-platform">{{ platform }}</span>
    </div>

    <!-- the table ---------------------------------------------------------- -->
    <section class="card">
      <div class="cap">
        <span>capabilities</span>
        <span class="tier t1">tier 1 · navigator.permissions</span>
        <span class="tier t3">tier 2 · appPermissions</span>
      </div>
      <div class="grid" id="perm-list">
        <div class="hrow">
          <span class="k">name</span>
          <span class="s">tier 1 · state</span>
          <span class="s">tier 2 · state</span>
          <span class="a"></span>
        </div>
        <div v-for="r in rows" :key="r.name" class="prow" :data-perm="r.name">
          <span class="k">{{ r.name }}</span>
          <span class="s" :class="cls(r.webState)">{{ r.webState }}</span>
          <span class="s" :class="cls(r.state)" :data-state="r.state">{{ r.state }}</span>
          <span class="a">
            <span
              v-if="r.canRequest"
              class="btn"
              :class="{ busy: r.requesting }"
              @click="request(r)"
            >{{ r.requesting ? "waiting…" : "Request" }}</span>
            <span v-else class="pill">{{ r.state === "granted" ? "nothing to ask" : "no one to ask" }}</span>
          </span>
        </div>
      </div>
      <div class="note" id="perm-last">{{ lastRequest || "no request made yet" }}</div>
    </section>

    <!-- change log --------------------------------------------------------- -->
    <section class="card">
      <div class="cap">change events</div>
      <div class="grid" id="perm-changes">
        <div v-for="(c, i) in changes" :key="i" class="v">{{ c }}</div>
        <div v-if="!changes.length" class="note">
          Nothing yet. Flip a toggle in Settings &gt; Privacy &amp; security &gt;
          Microphone while this page is open and a row will appear here — the
          host subscribes to the consent store with `RegNotifyChangeKeyValue`,
          so this is a real OS notification and not a poll.
        </div>
      </div>
    </section>

    <section class="card">
      <div class="cap">why the two columns differ</div>
      <div class="note">
        Tier 1 is the W3C API and only knows three words, so a capability this
        platform has no notion of has to read "denied" there. Tier 2 adds a
        fourth, "unsupported", and an advisory naming the platform — that is the
        compat-first rule: never diverge silently, and never widen a web enum to
        say something the web cannot say.
      </div>
      <div class="note">
        On Windows there is no "prompt" state at all. Consent is pre-answered in
        Settings before the app runs, so the honest answer is always granted or
        denied, and Request opens the right Settings page rather than raising a
        dialog this process is not allowed to raise.
      </div>
    </section>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.hd {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
}
.h1 {
  font-size: 16px;
  color: #ffffff;
}
.sub {
  font-size: 11px;
  color: #ffffff70;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-radius: 10px;
  background: #ffffff0d;
}
.cap {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: #ffffff80;
}
.tier {
  padding: 2px;
  border-radius: 4px;
  font-size: 10px;
}
.t1 {
  background: #4f8cff40;
  color: #b9d0ff;
}
.t3 {
  background: #ffb04f40;
  color: #ffd9a8;
}
.grid {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.hrow {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding: 4px;
}
.prow {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding: 4px;
  border-radius: 6px;
  background: #ffffff08;
}
.k {
  width: 150px;
  font-size: 12px;
  color: #e8eefc;
}
.s {
  width: 120px;
  font-size: 11px;
  color: #ffffff70;
}
.a {
  flex: 1;
  display: flex;
  flex-direction: row;
  justify-content: flex-end;
}
.ok {
  color: #86efac;
}
.no {
  color: #fca5a5;
}
.ask {
  color: #fcd34d;
}
.unk {
  color: #ffffff55;
}
.v {
  font-size: 12px;
  color: #e8eefc;
}
.note {
  font-size: 11px;
  color: #ffffff70;
}
.btn {
  padding: 5px;
  border-radius: 6px;
  font-size: 11px;
  color: #e8eefc;
  background: #ffffff14;
  cursor: pointer;
  transition: background-color 140ms ease;
}
.btn:hover {
  background: #ffffff2b;
}
.btn.busy {
  background: #ffb04f40;
  color: #ffd9a8;
}
.pill {
  padding: 4px;
  border-radius: 6px;
  font-size: 10px;
  color: #ffffff55;
  background: #ffffff08;
}
.pill.live {
  background: #4ade8040;
  color: #d7ffe4;
}
</style>
