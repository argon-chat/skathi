<script setup lang="ts">
import { computed, ref, watchEffect } from "vue";

type Todo = { id: number; text: string; done: boolean };

const draft = ref("");
const todos = ref<Todo[]>([]);
let nextId = 1;
const left = computed(() => todos.value.filter((t) => !t.done).length);

function add() {
  const text = draft.value.trim();
  if (!text) return;
  todos.value.push({ id: nextId++, text, done: false });
  draft.value = "";
}

watchEffect(() => {
  document.title = `Todos (${left.value})`;
});
</script>

<template>
  <main class="app">
    <div class="composer">
      <input v-model="draft" class="new-todo" placeholder="What needs doing?" @keydown.enter="add" />
      <button class="add" @click="add">Add</button>
    </div>
    <ul class="todos">
      <li v-for="todo in todos" :key="todo.id">
        <button class="todo" :class="{ done: todo.done }" @click="todo.done = !todo.done">{{ todo.text }}</button>
      </li>
    </ul>
    <p class="count">{{ left }} {{ left === 1 ? "item" : "items" }} left</p>
  </main>
</template>

<style scoped>
.app {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  color: #f3f3f3;
}
.composer {
  display: flex;
  gap: 8px;
}
.new-todo {
  flex: 1;
  padding: 6px 10px;
  border-radius: 6px;
  background: #ffffff14;
}
.add {
  padding: 6px 12px;
  border-radius: 6px;
  background: #ffffff24;
}
.todo {
  padding: 6px 0;
}
.todo.done {
  opacity: 0.5;
  text-decoration: line-through;
}
.count {
  font-size: 12px;
  color: #ffffffb0;
}
</style>
