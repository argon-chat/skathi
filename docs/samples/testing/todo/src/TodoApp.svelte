<script lang="ts">
  type Todo = { id: number; text: string; done: boolean };

  let draft = $state("");
  let todos = $state<Todo[]>([]);
  let nextId = 1;
  const left = $derived(todos.filter((t) => !t.done).length);

  function add() {
    const text = draft.trim();
    if (!text) return;
    todos.push({ id: nextId++, text, done: false });
    draft = "";
  }

  $effect(() => {
    document.title = `Todos (${left})`;
  });
</script>

<main class="app">
  <div class="composer">
    <input
      bind:value={draft}
      class="new-todo"
      placeholder="What needs doing?"
      onkeydown={(e) => e.key === "Enter" && add()}
    />
    <button class="add" onclick={add}>Add</button>
  </div>
  <ul class="todos">
    {#each todos as todo (todo.id)}
      <li>
        <button class="todo" class:done={todo.done} onclick={() => (todo.done = !todo.done)}>{todo.text}</button>
      </li>
    {/each}
  </ul>
  <p class="count">{left} {left === 1 ? "item" : "items"} left</p>
</main>

<style>
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
