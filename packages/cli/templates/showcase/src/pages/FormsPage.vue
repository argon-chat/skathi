<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const name = ref("");
const plan = ref("team");
const seats = ref(5);
const yearly = ref(true);
const notes = ref("");
const price = () => seats.value * (plan.value === "team" ? 8 : 15) * (yearly.value ? 10 : 1);

const toppings = ["Cheese", "Mushrooms", "Olives", "Basil"];
const chosen = ref<string[]>(["Cheese"]);
const all = computed(() => chosen.value.length === toppings.length);
const some = computed(() => chosen.value.length > 0 && !all.value);
const toggleAll = () => (chosen.value = all.value ? [] : [...toppings]);
const size = ref("medium");

// Selection is the document's, as in a browser: drag over the paragraph, or double-click a word.
const selected = ref("");
const onSelection = () => (selected.value = getSelection()?.toString() ?? "");
onMounted(() => document.addEventListener("selectionchange", onSelection));
onUnmounted(() => document.removeEventListener("selectionchange", onSelection));
</script>

<template>
  <Page title="Forms" lead="Native text editing with IME, undo and the system caret, driven by plain v-model.">
    <Card title="Controls" api="v-model">
      <div class="form">
        <label class="field">
          <span class="label">Name</span>
          <input v-model="name" class="input" placeholder="Ada Lovelace" />
        </label>
        <label class="field">
          <span class="label">Plan</span>
          <select v-model="plan" class="input">
            <option value="team">Team</option>
            <option value="business">Business</option>
          </select>
        </label>
        <label class="field">
          <span class="label">Seats · {{ seats }}</span>
          <input v-model.number="seats" type="range" min="1" max="50" />
        </label>
        <div class="check">
          <button
            class="switch"
            :class="{ on: yearly }"
            role="switch"
            :aria-checked="yearly"
            aria-label="Pay yearly"
            @click="yearly = !yearly"
          >
            <span class="knob" />
          </button>
          <span>Pay yearly (two months free)</span>
        </div>
        <label class="field">
          <span class="label">Notes</span>
          <textarea v-model="notes" class="input notes" placeholder="Enter adds a line" />
        </label>
      </div>
      <div class="summary">
        {{ name || "Someone" }} · {{ plan }} · {{ seats }} seats · ${{ price() }} per {{ yearly ? "year" : "month" }}
      </div>
    </Card>

    <Card title="Checkboxes and radios" api="<input type=checkbox> · :checked">
      <div class="choices">
        <div class="group">
          <label class="choice">
            <input type="checkbox" :checked="all" :indeterminate="some" @change="toggleAll" />
            <span>All toppings</span>
          </label>
          <label v-for="t in toppings" :key="t" class="choice nested">
            <input v-model="chosen" type="checkbox" :value="t" />
            <span>{{ t }}</span>
          </label>
        </div>
        <div class="group">
          <label v-for="s in ['small', 'medium', 'large']" :key="s" class="choice">
            <input v-model="size" type="radio" name="size" :value="s" />
            <span>{{ s }}</span>
          </label>
        </div>
      </div>
      <div class="summary">A {{ size }} pizza with {{ chosen.join(", ").toLowerCase() || "nothing on it" }}</div>
      <p class="faint">
        Native controls with v-model, drawn in accent-color. Space and arrow keys work; the labels turn bold through
        :checked.
      </p>
    </Card>

    <Card title="Text selection" api="getSelection()">
      <p class="prose">
        Text in a Skathi window is selectable like text on a web page. Drag across this paragraph, double-click a word
        or press Ctrl+A, then Ctrl+C: the selection goes to the system clipboard.
      </p>
      <p class="faint">Selected: {{ selected || "nothing" }}</p>
      <p class="prose locked">user-select: none keeps this line out of every selection.</p>
    </Card>
  </Page>
</template>

<style scoped>
.form {
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 420px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.label {
  font-size: 12px;
  color: var(--text-faint);
}
.check {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
}
.switch {
  display: flex;
  width: 36px;
  height: 20px;
  padding: 2px;
  border-radius: 10px;
  background: var(--surface-strong);
  cursor: pointer;
  transition: background-color 160ms ease;
}
.switch.on {
  background: var(--accent-strong);
}
.knob {
  width: 16px;
  height: 16px;
  border-radius: 8px;
  background: #ffffff;
  transition: margin-left 160ms ease;
}
.switch.on .knob {
  margin-left: 16px;
}
.notes {
  height: 80px;
  padding: 8px 12px;
}
.summary {
  padding: 12px;
  border-radius: 10px;
  background: var(--accent-soft);
  font-size: 13px;
}
.choices {
  display: flex;
  gap: 48px;
  accent-color: var(--accent-strong);
}
.group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.choice {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--text-muted);
  cursor: pointer;
}
.nested {
  margin-left: 22px;
}
.choice input:checked + span {
  font-weight: 600;
  color: var(--text);
}
.prose {
  margin: 0;
  font-size: 14px;
  line-height: 22px;
  color: var(--text);
}
.locked {
  user-select: none;
  color: var(--text-faint);
}
</style>
