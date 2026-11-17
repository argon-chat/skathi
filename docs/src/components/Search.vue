<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { navigate as navigateTo } from "astro:transitions/client";

interface Entry {
  title: string;
  description: string;
  url: string;
  body: string;
  status: string;
}
interface SearchResult {
  id: string;
  url: string;
  title: string;
  status: string;
  excerpt: string;
}
interface Pagefind {
  search(query: string): Promise<{
    results: {
      id: string;
      data(): Promise<{ url: string; meta: { title: string; status?: string }; excerpt: string }>;
    }[];
  }>;
}

const props = defineProps<{ entries: Entry[] }>();
const dialog = ref<HTMLDialogElement>();
const input = ref<HTMLInputElement>();
const query = ref("");
const results = ref<SearchResult[]>([]);
const active = ref(0);
const loading = ref(false);
const error = ref(false);
const hasQuery = computed(() => query.value.trim().length > 0);
let pagefind: Promise<Pagefind> | undefined;
let request = 0;
let debounce: ReturnType<typeof setTimeout> | undefined;

async function open(): Promise<void> {
  dialog.value?.showModal();
  await nextTick();
  input.value?.focus();
}

function close(): void {
  dialog.value?.close();
}

function onShortcut(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    if (dialog.value?.open) close();
    else void open();
  }
}

async function search(term: string, token: number): Promise<void> {
  loading.value = true;
  error.value = false;
  try {
    let found: SearchResult[];
    if (import.meta.env.DEV) {
      const words = term.toLowerCase().split(/\s+/);
      found = props.entries
        .filter((entry) =>
          words.every((word) => `${entry.title} ${entry.description} ${entry.body}`.toLowerCase().includes(word)),
        )
        .slice(0, 12)
        .map((entry) => ({
          id: entry.url,
          url: entry.url,
          title: entry.title,
          status: entry.status,
          excerpt: entry.description,
        }));
    } else {
      const moduleUrl = "/pagefind/pagefind.js";
      pagefind ??= import(/* @vite-ignore */ moduleUrl) as Promise<Pagefind>;
      const foundPages = await (await pagefind).search(term);
      found = await Promise.all(
        foundPages.results.slice(0, 12).map(async (result) => {
          const data = await result.data();
          return {
            id: result.id,
            url: data.url,
            title: data.meta.title,
            status: data.meta.status ?? "",
            excerpt: data.excerpt.replace(/<[^>]+>/g, ""),
          };
        }),
      );
    }
    if (token === request) {
      results.value = found;
      active.value = 0;
    }
  } catch {
    if (token === request) {
      error.value = true;
      pagefind = undefined;
    }
  } finally {
    if (token === request) loading.value = false;
  }
}

watch(query, (value) => {
  clearTimeout(debounce);
  const token = ++request;
  if (!value.trim()) {
    results.value = [];
    loading.value = false;
    error.value = false;
    return;
  }
  loading.value = true;
  debounce = setTimeout(() => {
    void search(value.trim(), token);
  }, 160);
});

function navigate(event: KeyboardEvent): void {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (results.value.length)
      active.value =
        (active.value + (event.key === "ArrowDown" ? 1 : -1) + results.value.length) % results.value.length;
    document.getElementById(`search-result-${active.value}`)?.scrollIntoView({ block: "nearest" });
  }
  const result = results.value[active.value];
  if (event.key === "Enter" && result) {
    event.preventDefault();
    void navigateTo(result.url);
  }
}

onMounted(() => {
  window.addEventListener("keydown", onShortcut);
  window.addEventListener("skathi:search", open);
});
onUnmounted(() => {
  clearTimeout(debounce);
  window.removeEventListener("keydown", onShortcut);
  window.removeEventListener("skathi:search", open);
});
</script>

<template>
  <button class="search-trigger" aria-label="Search documentation" @click="open">
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      aria-hidden="true"
    >
      <circle cx="10.5" cy="10.5" r="7" />
      <path d="m16 16 5 5" />
    </svg>
    <span>Search docs…</span><kbd>Ctrl K</kbd>
  </button>
  <dialog
    ref="dialog"
    class="search-dialog"
    aria-labelledby="search-title"
    @keydown.esc.prevent.stop="close"
    @click="$event.target === dialog && close()"
  >
    <div class="search-panel">
      <h2 id="search-title" class="sr-only">Search documentation</h2>
      <div class="search-input-row">
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          aria-hidden="true"
        >
          <circle cx="10.5" cy="10.5" r="7" />
          <path d="m16 16 5 5" />
        </svg>
        <input
          ref="input"
          v-model="query"
          type="search"
          placeholder="What would you like to build?"
          aria-label="Search documentation"
          aria-controls="search-results"
          :aria-activedescendant="results.length ? `search-result-${active}` : undefined"
          @keydown="navigate"
        />
        <button class="search-close" aria-label="Close search" @click="close"><kbd>Esc</kbd></button>
      </div>
      <div id="search-results" class="search-results" :aria-busy="loading">
        <p v-if="!hasQuery" class="search-message">Find a guide, an API, or your next step.</p>
        <p v-else-if="loading" class="search-message" role="status">Searching documentation…</p>
        <p v-else-if="error" class="search-message" role="status">
          Search couldn’t load. Try again, or browse the sidebar.
        </p>
        <p v-else-if="!results.length" class="search-message" role="status">
          No results for “{{ query }}”. Try a different term.
        </p>
        <template v-else>
          <p class="sr-only" role="status">{{ results.length }} results found.</p>
          <a
            v-for="(result, index) in results"
            :id="`search-result-${index}`"
            :key="result.id"
            :href="result.url"
            class="search-result"
            :class="{ 'is-active': index === active }"
            @pointerenter="active = index"
          >
            <span
              >{{ result.title }}<span v-if="result.status" class="search-status">{{ result.status }}</span></span
            ><small>{{ result.excerpt }}</small>
          </a>
        </template>
      </div>
      <div class="search-footer">
        <span><kbd>↑</kbd> <kbd>↓</kbd> to navigate <kbd>↵</kbd> to open</span><span>Skathi docs</span>
      </div>
    </div>
  </dialog>
</template>
