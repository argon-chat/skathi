<script setup lang="ts">
import { appStorage } from "@skathi-native/runtime";
import { onMounted, ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";
import { appName } from "../product";

const visits = ref(0);
const note = ref("");
onMounted(() => {
  visits.value = Number(localStorage.getItem("visits") ?? 0) + 1;
  localStorage.setItem("visits", String(visits.value));
  note.value = localStorage.getItem("note") ?? "";
});
const saveNote = () => localStorage.setItem("note", note.value);

interface Todo {
  id: number;
  text: string;
}
const todos = ref<Todo[]>([]);
const draft = ref("");
const status = ref("");

const done = <T,>(request: IDBRequest<T>) =>
  new Promise<T>((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });

let db: IDBDatabase | null = null;
async function store(mode: "readonly" | "readwrite") {
  if (!db) {
    const open = indexedDB.open(appName, 1);
    open.onupgradeneeded = () => open.result.createObjectStore("todos", { keyPath: "id", autoIncrement: true });
    db = await done(open);
  }
  return db.transaction("todos", mode).objectStore("todos");
}

async function update(change?: (todos: IDBObjectStore) => IDBRequest) {
  try {
    if (change) await done(change(await store("readwrite")));
    todos.value = await done((await store("readonly")).getAll() as IDBRequest<Todo[]>);
  } catch (e) {
    status.value = describe(e);
  }
}
onMounted(() => update());

function add() {
  const text = draft.value.trim();
  if (!text) return;
  draft.value = "";
  update((s) => s.add({ text }));
}
</script>

<template>
  <Page title="Storage" lead="The web's storage APIs, kept on disk in the app's own data folder between runs.">
    <Card title="localStorage" api="localStorage">
      <p class="muted">This page was opened {{ visits }} times, counting earlier runs.</p>
      <input v-model="note" class="input" placeholder="A note that survives a restart" @input="saveNote" />
    </Card>

    <Card title="IndexedDB" api="indexedDB">
      <div class="row">
        <input v-model="draft" class="input grow" placeholder="Something to do" @keyup.enter="add" />
        <button class="btn primary" @click="add">Add</button>
      </div>
      <div v-for="todo in todos" :key="todo.id" class="todo">
        <span>{{ todo.text }}</span>
        <button class="btn" @click="update((s) => s.delete(todo.id))">Done</button>
      </div>
      <p v-if="!todos.length" class="faint">Nothing yet. Items are rows in a SQLite file under the data folder.</p>
      <p v-if="status" class="faint">{{ status }}</p>
    </Card>

    <Card title="Where it goes" api="appStorage">
      <div class="kv">
        <span class="k">folder</span><span class="v mono">{{ appStorage.folder || "in memory" }}</span>
      </div>
      <div class="kv">
        <span class="k">encrypted</span><span class="v">{{ appStorage.encrypted ? "yes" : "no" }}</span>
      </div>
    </Card>
  </Page>
</template>

<style scoped>
.grow {
  flex: 1;
}
.todo {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 8px 8px 12px;
  border-radius: 9px;
  background: var(--surface-hover);
  font-size: 13px;
}
</style>
