<script setup lang="ts">
import { computed, onActivated, onDeactivated, onMounted, onUnmounted, ref } from "vue";
import { useSession } from "../stores/session";

const session = useSession();

const name = ref("");
const notes = ref("");
const term = ref("");
const prose = ref("");
const noteLines = computed(() => notes.value.split("\n").length);
const focused = ref("nothing");

const submitted = ref<string[]>([]);
const submit = () => {
  const v = name.value.trim();
  if (!v) return;
  submitted.value = [...submitted.value.slice(-2), v];
  session.addNote(v);
  name.value = "";
};
const ellipsis = (s: string) => (s.length > 14 ? s.slice(0, 14) + "…" : s);
const submittedText = computed(() => submitted.value.map(ellipsis).join(" · ") || "—");

const blocked = ref(0);
const lastKey = ref("—");
const guardDigits = (e: VnKeyboardEvent) => {
  lastKey.value = JSON.stringify(e.key);
  if (e.key.length === 1 && e.key >= "0" && e.key <= "9") {
    e.preventDefault();
    blocked.value++;
  }
};

const activations = ref(0);
console.log("[app] FormsPage setup ran");
onMounted(() => console.log("[app] FormsPage mounted"));
onUnmounted(() => console.log("[app] FormsPage unmounted"));
onActivated(() => {
  activations.value++;
  console.log("[app] FormsPage ACTIVATED (#" + activations.value + ")");
});
onDeactivated(() => console.log("[app] FormsPage DEACTIVATED"));
</script>

<template>
  <div class="card">
    <div class="title">Forms — native IME editor driven by stock v-model</div>
    <input
      class="field"
      v-model="name"
      tabindex="1"
      placeholder="type here, press Enter"
      @keydown="guardDigits"
      @keydown.enter="submit"
      @focus="focused = 'input'"
      @blur="focused = 'nothing'"
    />
    <div class="hint">v-model echo: "{{ name }}" ({{ name.length }} chars)</div>
    <textarea
      class="field notes"
      v-model="notes"
      rows="3"
      tabindex="2"
      placeholder="multi-line — Enter inserts a newline, it does not submit"
      @focus="focused = 'textarea'"
      @blur="focused = 'nothing'"
    />
    <div class="hint">
      textarea v-model: {{ noteLines }} line(s), {{ notes.length }} chars — survives a tab switch
    </div>

    <input
      class="field search"
      v-model="term"
      type="search"
      tabindex="3"
      autocapitalize="none"
      autocorrect="off"
      spellcheck="false"
      placeholder="search — no autocorrect, Search confirm key"
      @focus="focused = 'search'"
      @blur="focused = 'nothing'"
    />
    <input
      class="field prose"
      v-model="prose"
      tabindex="4"
      autocapitalize="sentences"
      autocorrect="on"
      spellcheck="true"
      enterkeyhint="send"
      placeholder="prose — assistance on, Send confirm key"
      @focus="focused = 'prose'"
      @blur="focused = 'nothing'"
    />
    <div class="hint">
      the two fields above declare different text assistance; `logs` shows the
      TextInputConfiguration gpui was handed when each takes focus
    </div>
    <div class="hint">Enter-submitted: {{ submittedText }}</div>
    <div class="hint">last e.key: {{ lastKey }} — digits blocked by preventDefault: {{ blocked }}</div>
    <div class="hint">focus → {{ focused }}</div>
    <div class="hint keep">
      &lt;KeepAlive&gt;: setup ran once · onActivated fired {{ activations }}× — the text above
      survives a tab switch
    </div>

    <div class="store">
      <div class="store-title">pinia store "session" — shared across every tab</div>
      <div class="store-row">
        <div class="btn" @click="session.bump()">visits++ ({{ session.visits }})</div>
        <div class="btn ghost" @click="session.reset()">reset</div>
        <div class="btn ghost" @click="session.setUser(session.user === 'guest' ? 'yuuki' : 'guest')">
          user = {{ session.user }}
        </div>
      </div>
      <div class="hint">store.badge (a computed inside the store): {{ session.badge }}</div>
      <div class="hint">
        store.notes ({{ session.noteCount }}): {{ session.notes.join(" · ") || "—" }} — Enter above
        appends here too
      </div>
    </div>
  </div>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 14px;
  color: #ffffff;
}
.field {
  height: 32px;
  width: 280px;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  font-size: 13px;
  color: #ffffff;
}
.field:focus {
  border-color: #7fd4ff;
}
.notes {
  height: auto;
  width: 320px;
  line-height: 18px;
}
.search,
.prose {
  width: 320px;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
.keep {
  color: #a5f3a5;
}

.store {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: #ffd4790f;
  border: 1px solid #ffd47933;
}
.store-title {
  font-size: 12px;
  color: #ffd479;
}
.store-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.btn:hover {
  background: #4f8cffcc;
}
.btn.ghost {
  background: #ffffff0d;
  border: 1px solid #ffffff2e;
  color: #ffffffcc;
}
.btn.ghost:hover {
  background: #ffffff22;
}
</style>
