<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import "../styles/search.css";
import { SearchIndex, type SearchData, type Segment } from "../lib/search";

type Icon = "page" | "hash" | "api" | "css" | "config" | "recent" | "link";

interface Item {
  key: string;
  title: Segment[];
  crumb: string;
  url: string;
  snippet: Segment[];
  kind: string;
  status: string;
  icon: Icon;
}

interface ItemGroup {
  label: string;
  items: Item[];
}

interface Recent {
  title: string;
  crumb: string;
  url: string;
}

const recentKey = "skathi-search-recent";
const starters: Recent[] = [
  { title: "Quick start", crumb: "Getting started", url: "/quick-start/" },
  { title: "Windows", crumb: "Guides", url: "/guides/windows/" },
  { title: "Window API", crumb: "Reference", url: "/window-api/" },
  { title: "App APIs", crumb: "Reference", url: "/app-api/" },
  { title: "CSS coverage", crumb: "Reference", url: "/css/" },
  { title: "Web platform coverage", crumb: "Reference", url: "/web-platform/" },
];
const tones: Readonly<Record<string, string>> = {
  Partial: "partial",
  Planned: "planned",
  "On hold": "hold",
  "Not planned": "no",
  Deprecated: "gone",
  Experimental: "planned",
};

let loader: Promise<SearchIndex> | undefined;

function loadIndex(): Promise<SearchIndex> {
  loader ??= fetch("/search-index.json")
    .then(async (response) => {
      if (!response.ok) throw new Error(`Search index: HTTP ${response.status}`);
      return new SearchIndex((await response.json()) as SearchData);
    })
    .catch((error: unknown) => {
      loader = undefined;
      throw error;
    });
  return loader;
}

const dialog = ref<HTMLDialogElement>();
const input = ref<HTMLInputElement>();
const list = ref<HTMLElement>();
const query = ref("");
const index = shallowRef<SearchIndex>();
const failed = ref(false);
const active = ref(0);
const modifier = ref("Ctrl");
const recents = ref<Recent[]>([]);
const trimmed = computed(() => query.value.trim());

function plainItem(entry: Recent, icon: Icon, prefix: string): Item {
  return {
    key: `${prefix}${entry.url}`,
    title: [{ text: entry.title, hit: false }],
    crumb: entry.crumb,
    url: entry.url,
    snippet: [],
    kind: "",
    status: "",
    icon,
  };
}

function iconFor(group: string, kind: string, section: boolean): Icon {
  if (section) return "hash";
  if (!kind) return "page";
  if (group === "CSS") return "css";
  if (group === "sk.config.json") return "config";
  return "api";
}

const groups = computed<ItemGroup[]>(() => {
  if (!trimmed.value) {
    const recent = recents.value.map((entry) => plainItem(entry, "recent", "recent:"));
    const start = starters.map((entry) => plainItem(entry, "link", "start:"));
    return [...(recent.length ? [{ label: "Recent", items: recent }] : []), { label: "Start here", items: start }];
  }
  if (!index.value) return [];
  return index.value.search(trimmed.value).map((group) => ({
    label: group.label,
    items: group.hits.map((hit) => ({
      key: String(hit.id),
      title: hit.title,
      crumb: hit.crumb,
      url: hit.url,
      snippet: hit.snippet,
      kind: hit.kind,
      status: hit.status,
      icon: iconFor(hit.group, hit.kind, hit.section),
    })),
  }));
});
const items = computed(() => groups.value.flatMap((group) => group.items));
const offsets = computed(() => {
  let offset = 0;
  return groups.value.map((group) => {
    const start = offset;
    offset += group.items.length;
    return start;
  });
});
const announcement = computed(() => {
  if (!trimmed.value) return "";
  if (failed.value) return "Search could not load.";
  if (!index.value) return "Loading search.";
  const count = items.value.length;
  return count ? `${count} result${count === 1 ? "" : "s"} for ${trimmed.value}.` : `No results for ${trimmed.value}.`;
});

watch(groups, () => {
  active.value = 0;
  list.value?.scrollTo({ top: 0 });
});

function optionId(position: number): string {
  return `search-option-${position}`;
}

async function warm(): Promise<void> {
  if (index.value) return;
  try {
    index.value = await loadIndex();
    failed.value = false;
  } catch {
    failed.value = true;
  }
}

function readRecents(): Recent[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(recentKey) ?? "[]");
    if (!Array.isArray(stored)) return [];
    return stored.filter(
      (entry): entry is Recent =>
        typeof entry === "object" &&
        entry !== null &&
        typeof (entry as Recent).title === "string" &&
        typeof (entry as Recent).crumb === "string" &&
        typeof (entry as Recent).url === "string" &&
        (entry as Recent).url.startsWith("/"),
    );
  } catch {
    return [];
  }
}

function remember(item: Item): void {
  const entry: Recent = { title: item.title.map((part) => part.text).join(""), crumb: item.crumb, url: item.url };
  recents.value = [entry, ...recents.value.filter((recent) => recent.url !== entry.url)].slice(0, 5);
  try {
    localStorage.setItem(recentKey, JSON.stringify(recents.value));
  } catch {
    // Recents are optional where storage is blocked.
  }
}

async function open(): Promise<void> {
  if (!dialog.value || dialog.value.open) return;
  recents.value = readRecents();
  dialog.value.showModal();
  document.documentElement.classList.add("search-open");
  await nextTick();
  input.value?.focus();
  input.value?.select();
  void warm();
}

function close(): void {
  dialog.value?.close();
}

function onClose(): void {
  document.documentElement.classList.remove("search-open");
}

function choose(item: Item, event?: MouseEvent | KeyboardEvent): void {
  if (!item.key.startsWith("recent:")) remember(item);
  if (event?.ctrlKey || event?.metaKey || event?.shiftKey) {
    if (event instanceof KeyboardEvent) window.open(item.url, "_blank", "noopener");
    return;
  }
  close();
  if (event instanceof KeyboardEvent) document.getElementById(optionId(active.value))?.click();
}

function move(step: number): void {
  const count = items.value.length;
  if (!count) return;
  active.value = (active.value + step + count) % count;
  void nextTick(() => document.getElementById(optionId(active.value))?.scrollIntoView({ block: "nearest" }));
}

function onKey(event: KeyboardEvent): void {
  if (event.isComposing) return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    move(event.key === "ArrowDown" ? 1 : -1);
  } else if (event.key === "PageDown" || event.key === "PageUp") {
    event.preventDefault();
    move(event.key === "PageDown" ? 5 : -5);
  } else if (event.key === "Enter") {
    const item = items.value[active.value];
    if (!item) return;
    event.preventDefault();
    choose(item, event);
  }
}

function typing(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))
  );
}

function onShortcut(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === "k") {
    event.preventDefault();
    if (dialog.value?.open) close();
    else void open();
    return;
  }
  // Reference catalogs bind "/" to their own filter.
  if (event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey || typing(event.target)) return;
  if (dialog.value?.open || document.querySelector(".part-search input")) return;
  event.preventDefault();
  void open();
}

function onDialogClick(event: MouseEvent): void {
  if (event.target === dialog.value) close();
}

onMounted(() => {
  if (/Mac|iPhone|iPad/.test(navigator.platform)) modifier.value = "⌘";
  window.addEventListener("keydown", onShortcut);
  window.addEventListener("skathi:search", open);
});
onUnmounted(() => {
  document.documentElement.classList.remove("search-open");
  window.removeEventListener("keydown", onShortcut);
  window.removeEventListener("skathi:search", open);
});
</script>

<template>
  <button class="search-trigger" type="button" aria-haspopup="dialog" @click="open" @pointerenter="warm" @focus="warm">
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
    <span>Search docs…</span><kbd aria-hidden="true">{{ modifier }} K</kbd>
  </button>
  <dialog ref="dialog" class="search-dialog" aria-label="Search documentation" @close="onClose" @click="onDialogClick">
    <div class="search-panel">
      <div class="search-bar">
        <svg
          class="search-bar-icon"
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
          type="text"
          role="combobox"
          aria-label="Search guides, APIs and CSS"
          aria-autocomplete="list"
          aria-controls="search-listbox"
          :aria-expanded="items.length > 0"
          :aria-activedescendant="items.length ? optionId(active) : undefined"
          placeholder="Search guides, APIs, CSS…"
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          enterkeyhint="go"
          @keydown="onKey"
        />
        <button
          v-if="query"
          class="search-clear"
          type="button"
          aria-label="Clear search"
          @click="
            query = '';
            input?.focus();
          "
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </button>
        <button class="search-cancel" type="button" aria-label="Close search" @click="close">
          <kbd>Esc</kbd><span>Cancel</span>
        </button>
      </div>
      <div ref="list" class="search-body">
        <p class="sr-only" role="status">{{ announcement }}</p>
        <div v-if="trimmed && failed" class="search-message">
          <p>Search couldn’t load.</p>
          <button type="button" class="search-retry" @click="warm">Try again</button>
        </div>
        <div v-else-if="trimmed && !index" class="search-message search-loading" aria-hidden="true">
          <span></span><span></span><span></span>
        </div>
        <div v-else-if="trimmed && !items.length" class="search-message">
          <p>
            No results for <strong>“{{ trimmed }}”</strong>
          </p>
          <p class="search-hint">
            Try a shorter word, or browse the <a href="/window-api/" @click="close">Window API</a>,
            <a href="/app-api/" @click="close">App APIs</a> or <a href="/css/" @click="close">CSS coverage</a>.
          </p>
        </div>
        <div v-show="items.length" id="search-listbox" role="listbox" aria-label="Search results">
          <div
            v-for="(group, groupIndex) in groups"
            :key="group.label"
            class="search-group"
            role="group"
            :aria-labelledby="`search-group-${groupIndex}`"
          >
            <div :id="`search-group-${groupIndex}`" class="search-group-label">{{ group.label }}</div>
            <a
              v-for="(item, itemIndex) in group.items"
              :id="optionId((offsets[groupIndex] ?? 0) + itemIndex)"
              :key="item.key"
              :href="item.url"
              class="search-hit"
              role="option"
              tabindex="-1"
              :aria-selected="active === (offsets[groupIndex] ?? 0) + itemIndex"
              @pointermove="active = (offsets[groupIndex] ?? 0) + itemIndex"
              @click="choose(item, $event)"
            >
              <span class="search-hit-icon" :data-icon="item.icon" aria-hidden="true">
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.6"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path v-if="item.icon === 'page'" d="M7 3h7l5 5v13H7zM14 3v5h5M10 13h6M10 17h4" />
                  <path v-else-if="item.icon === 'hash'" d="M9 4 7 20M17 4l-2 16M4 9h16M3 15h16" />
                  <path v-else-if="item.icon === 'api'" d="m8 8-4 4 4 4M16 8l4 4-4 4M13.5 6l-3 12" />
                  <path
                    v-else-if="item.icon === 'css'"
                    d="M8 4c-2 0-2 1.5-2 3v2c0 1.5-1 3-2 3 1 0 2 1.5 2 3v2c0 1.5 0 3 2 3M16 4c2 0 2 1.5 2 3v2c0 1.5 1 3 2 3-1 0-2 1.5-2 3v2c0 1.5 0 3-2 3"
                  />
                  <path v-else-if="item.icon === 'config'" d="M4 7h10M18 7h2M4 17h4M12 17h8M14 5v4M8 15v4" />
                  <path v-else-if="item.icon === 'recent'" d="M12 7v5l3 2M3.5 12a8.5 8.5 0 1 0 2.5-6M3 4v4h4" />
                  <path v-else d="M5 12h14M13 6l6 6-6 6" />
                </svg>
              </span>
              <span class="search-hit-main">
                <span class="search-hit-title"
                  ><template v-for="(part, i) in item.title" :key="i"
                    ><mark v-if="part.hit">{{ part.text }}</mark
                    ><template v-else>{{ part.text }}</template></template
                  ></span
                >
                <span class="search-hit-crumb">{{ item.crumb }}</span>
                <span v-if="item.snippet.length" class="search-hit-snippet"
                  ><template v-for="(part, i) in item.snippet" :key="i"
                    ><mark v-if="part.hit">{{ part.text }}</mark
                    ><template v-else>{{ part.text }}</template></template
                  ></span
                >
              </span>
              <span v-if="item.kind || item.status" class="search-hit-tags">
                <span v-if="item.kind" class="search-tag">{{ item.kind }}</span>
                <span v-if="item.status" class="search-tag search-tag-status" :data-tone="tones[item.status]">{{
                  item.status
                }}</span>
              </span>
              <svg
                class="search-hit-enter"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.6"
                aria-hidden="true"
              >
                <path d="M20 5v7a3 3 0 0 1-3 3H5M9 11l-4 4 4 4" />
              </svg>
            </a>
          </div>
        </div>
      </div>
      <div class="search-footer" aria-hidden="true">
        <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span><span><kbd>↵</kbd> open</span
        ><span
          ><kbd>{{ modifier }} ↵</kbd> new tab</span
        ><span class="search-footer-end"><kbd>Esc</kbd> close</span>
      </div>
    </div>
  </dialog>
</template>
