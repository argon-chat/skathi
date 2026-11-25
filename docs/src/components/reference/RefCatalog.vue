<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import {
  inline,
  isNew,
  isProseName,
  newLabel,
  platforms,
  type CatalogEntry,
  type Platform,
  type Status,
} from "../../lib/reference";
import PlatformMatrix from "./PlatformMatrix.vue";
import StatusMark from "./StatusMark.vue";

const props = defineProps<{
  entries: CatalogEntry[];
  categories: string[];
  notes: Record<string, string>;
  kinds: Record<string, string>;
  base: string;
  noun: string;
  current: string;
}>();

type StatusFilter = "all" | "works" | "partial" | "planned" | "missing";
const statusFilters: {
  id: StatusFilter;
  label: string;
  match: (status: Status) => boolean;
}[] = [
  { id: "all", label: "All", match: () => true },
  { id: "works", label: "Supported", match: (s) => s === "supported" },
  { id: "partial", label: "Partial", match: (s) => s === "partial" },
  {
    id: "planned",
    label: "Planned",
    match: (s) => s === "planned" || s === "on-hold",
  },
  {
    id: "missing",
    label: "Not planned",
    match: (s) => s === "not-planned" || s === "deprecated",
  },
];
const statusLabels: Record<Status, string> = {
  supported: "Supported",
  partial: "Partial",
  planned: "Planned",
  "on-hold": "On hold",
  "not-planned": "Not planned",
  deprecated: "Deprecated",
};

const query = ref("");
const status = ref<StatusFilter>("all");
const kind = ref("all");
const platform = ref<Platform | "all">("all");
const onlyChanged = ref(false);
const active = ref(-1);
const input = ref<HTMLInputElement>();
const list = ref<HTMLElement>();

const kindOptions = computed(() => {
  const present = new Map<string, number>();
  for (const entry of props.entries) present.set(entry.kind, (present.get(entry.kind) ?? 0) + 1);
  return [...present].sort((a, b) => b[1] - a[1]).map(([id, count]) => ({ id, count, label: props.kinds[id] ?? id }));
});

function categoryId(category: string): string {
  return category
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
}

function statusOf(entry: CatalogEntry): Status {
  return platform.value === "all" ? entry.status : entry.platforms[platform.value];
}

function score(entry: CatalogEntry, terms: string[]): number {
  const name = entry.name.toLowerCase();
  const aliases = (entry.aliases ?? []).join(" ").toLowerCase();
  const rest = `${entry.summary} ${entry.category}`.toLowerCase();
  let total = 0;
  for (const term of terms) {
    if (name === term) total += 100;
    else if (name.startsWith(term)) total += 60;
    else if (name.includes(term)) total += 40;
    else if (aliases.includes(term)) total += 25;
    else if (rest.includes(term)) total += 8;
    else return 0;
  }
  return total;
}

const terms = computed(() =>
  query.value
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter((term) => term.length > 0),
);

const filtered = computed(() => {
  const statusMatch = statusFilters.find((f) => f.id === status.value)?.match ?? (() => true);
  return props.entries.filter(
    (entry) =>
      statusMatch(statusOf(entry)) &&
      (kind.value === "all" || entry.kind === kind.value) &&
      (!onlyChanged.value || isNew(entry.changed)),
  );
});

const results = computed(() =>
  terms.value.length
    ? filtered.value
        .map((entry) => ({ entry, score: score(entry, terms.value) }))
        .filter((hit) => hit.score > 0)
        .sort((a, b) => b.score - a.score || a.entry.name.localeCompare(b.entry.name))
        .map((hit) => hit.entry)
    : [],
);

const groups = computed(() =>
  props.categories
    .map((category) => ({
      category,
      id: categoryId(category),
      note: props.notes[category],
      items: filtered.value.filter((e) => e.category === category),
    }))
    .filter((group) => group.items.length > 0),
);

const visible = computed(() => (terms.value.length ? results.value : filtered.value));
const counts = computed(() =>
  Object.fromEntries(statusFilters.map((f) => [f.id, props.entries.filter((e) => f.match(statusOf(e))).length])),
);
const changedCount = computed(() => props.entries.filter((e) => isNew(e.changed)).length);
const filtersActive = computed(
  () =>
    status.value !== "all" ||
    kind.value !== "all" ||
    platform.value !== "all" ||
    onlyChanged.value ||
    terms.value.length > 0,
);
const signature = computed(
  () => `${terms.value.join(" ")}|${status.value}|${kind.value}|${platform.value}|${onlyChanged.value}`,
);

function reset(): void {
  query.value = "";
  status.value = "all";
  kind.value = "all";
  platform.value = "all";
  onlyChanged.value = false;
  input.value?.focus();
}

function highlight(name: string): { text: string; hit: boolean }[] {
  const term = terms.value.find((t) => name.toLowerCase().includes(t));
  if (!term) return [{ text: name, hit: false }];
  const at = name.toLowerCase().indexOf(term);
  return [
    { text: name.slice(0, at), hit: false },
    { text: name.slice(at, at + term.length), hit: true },
    { text: name.slice(at + term.length), hit: false },
  ].filter((part) => part.text.length > 0);
}

function rows(): HTMLAnchorElement[] {
  return [...(list.value?.querySelectorAll<HTMLAnchorElement>(".catalog-row") ?? [])];
}

function onKey(event: KeyboardEvent): void {
  const all = rows();
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    active.value = Math.min(Math.max(active.value + (event.key === "ArrowDown" ? 1 : -1), 0), all.length - 1);
    all[active.value]?.scrollIntoView({ block: "nearest" });
  } else if (event.key === "Enter") {
    const target = all[active.value] ?? (all.length === 1 ? all[0] : undefined);
    if (target) window.location.href = target.href;
  } else if (event.key === "Escape") {
    if (query.value) query.value = "";
    else input.value?.blur();
  }
}

function onGlobalKey(event: KeyboardEvent): void {
  const target = event.target as HTMLElement | null;
  if (event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) return;
  if (target && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) return;
  event.preventDefault();
  input.value?.focus();
  input.value?.select();
}

watch(signature, async () => {
  active.value = terms.value.length ? 0 : -1;
  const params = new URLSearchParams();
  if (query.value.trim()) params.set("q", query.value.trim());
  if (status.value !== "all") params.set("status", status.value);
  if (kind.value !== "all") params.set("kind", kind.value);
  if (platform.value !== "all") params.set("platform", platform.value);
  if (onlyChanged.value) params.set("changed", props.current);
  const search = params.toString();
  history.replaceState(
    history.state,
    "",
    `${window.location.pathname}${search ? `?${search}` : ""}${window.location.hash}`,
  );
  await nextTick();
  window.dispatchEvent(new Event("resize"));
});

onMounted(() => {
  const params = new URLSearchParams(window.location.search);
  query.value = params.get("q") ?? "";
  const s = params.get("status");
  if (statusFilters.some((f) => f.id === s)) status.value = s as StatusFilter;
  const k = params.get("kind");
  if (k && kindOptions.value.some((option) => option.id === k)) kind.value = k;
  const p = params.get("platform");
  const known = platforms.find((option) => option.id === p);
  if (known) platform.value = known.id;
  onlyChanged.value = params.get("changed") === props.current;
  window.addEventListener("keydown", onGlobalKey);
});
onUnmounted(() => window.removeEventListener("keydown", onGlobalKey));
</script>

<template>
  <section class="catalog" aria-label="Reference catalog">
    <div class="catalog-controls">
      <label class="part-search">
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <path d="m21 21-4.35-4.35M19 11a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z" />
        </svg>
        <span class="sr-only">Search {{ noun }}</span>
        <input
          ref="input"
          v-model="query"
          type="search"
          autocomplete="off"
          spellcheck="false"
          :placeholder="`Search ${entries.length} ${noun}…`"
          aria-controls="catalog-list"
          @keydown="onKey"
        />
        <kbd v-if="!query" aria-hidden="true">/</kbd>
        <button
          v-else
          type="button"
          class="part-search-clear"
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
            stroke-width="1.8"
            stroke-linecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </button>
      </label>
      <div class="filter-bar" role="group" aria-label="Filters">
        <div class="segmented" role="radiogroup" aria-label="Status">
          <button
            v-for="filter in statusFilters"
            :key="filter.id"
            type="button"
            role="radio"
            :aria-checked="status === filter.id"
            @click="status = filter.id"
          >
            <StatusMark v-if="filter.id === 'works'" status="supported" :size="13" />
            <StatusMark v-else-if="filter.id === 'partial'" status="partial" :size="13" />
            <StatusMark v-else-if="filter.id === 'planned'" status="planned" :size="13" />
            <StatusMark v-else-if="filter.id === 'missing'" status="not-planned" :size="13" />
            {{ filter.label }}<span class="count">{{ counts[filter.id] }}</span>
          </button>
        </div>
        <div class="toggles">
          <div class="segmented segmented-platform" role="radiogroup" aria-label="Platform">
            <button type="button" role="radio" :aria-checked="platform === 'all'" @click="platform = 'all'">
              Any OS
            </button>
            <button
              v-for="option in platforms"
              :key="option.id"
              type="button"
              role="radio"
              :aria-checked="platform === option.id"
              @click="platform = option.id"
            >
              {{ option.label }}
            </button>
          </div>
          <label class="select">
            <span class="sr-only">Kind</span>
            <select v-model="kind">
              <option value="all">Every kind</option>
              <option v-for="option in kindOptions" :key="option.id" :value="option.id">
                {{ option.label }} ({{ option.count }})
              </option>
            </select>
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              aria-hidden="true"
            >
              <path d="m6 9 6 6 6-6" />
            </svg>
          </label>
          <button
            v-if="changedCount"
            type="button"
            class="toggle"
            :aria-pressed="onlyChanged"
            @click="onlyChanged = !onlyChanged"
          >
            <span class="new-dot" aria-hidden="true"></span>Changed in {{ current
            }}<span class="count">{{ changedCount }}</span>
          </button>
        </div>
      </div>
      <p class="catalog-status" aria-live="polite">
        <template v-if="filtersActive">
          <strong>{{ visible.length }}</strong> of {{ entries.length }}
          <button type="button" class="link-button" @click="reset">Clear filters</button>
        </template>
        <template v-else
          ><strong>{{ entries.length }}</strong> entries in {{ categories.length }} groups</template
        >
      </p>
    </div>

    <div id="catalog-list" ref="list" :key="signature" class="catalog-list">
      <template v-if="terms.length">
        <h2 v-if="results.length" id="results" class="catalog-group-title">
          Results<span class="group-count">{{ results.length }}</span>
        </h2>
        <ul v-if="results.length" class="catalog-rows">
          <li v-for="(entry, index) in results" :key="entry.slug">
            <a
              :href="entry.link?.url ?? `${base}${entry.slug}/`"
              class="catalog-row"
              :class="{ 'is-active': index === active }"
            >
              <span class="row-main">
                <span class="row-name">
                  <span v-if="isProseName(entry.name)" class="row-name-text" v-html="inline(entry.name)"></span
                  ><code v-else
                    ><template v-for="(part, i) in highlight(entry.name)" :key="i"
                      ><mark v-if="part.hit">{{ part.text }}</mark
                      ><template v-else>{{ part.text }}</template></template
                    ></code
                  >
                  <span v-if="entry.skathiOnly" class="flag flag-skathi">Skathi</span>
                  <span v-if="entry.experimental" class="flag flag-experimental">Experimental</span>
                  <span v-if="entry.link" class="flag flag-link">{{ entry.link.title }}</span>
                  <span v-if="entry.fresh" class="flag flag-new" :title="entry.fresh">{{ newLabel }}</span>
                </span>
                <span class="row-summary"
                  ><span class="row-category">{{ entry.category }}</span
                  ><span v-html="inline(entry.summary)"></span
                ></span>
              </span>
              <PlatformMatrix
                v-if="entry.split && platform === 'all'"
                :platforms="entry.platforms"
                :since="entry.platformSince"
              />
              <span v-else class="row-status" :data-status="statusOf(entry)"
                ><StatusMark :status="statusOf(entry)" :size="14" />{{ statusLabels[statusOf(entry)] }}</span
              >
              <span class="row-since" :class="{ 'is-new': isNew(entry.changed) }">{{ entry.since ?? "" }}</span>
            </a>
          </li>
        </ul>
      </template>
      <template v-else>
        <section v-for="group in groups" :key="group.id" class="catalog-group">
          <h2 :id="group.id" class="catalog-group-title">
            {{ group.category }}<span class="group-count">{{ group.items.length }}</span>
          </h2>
          <p v-if="group.note" class="group-note" v-html="group.note"></p>
          <ul class="catalog-rows">
            <li v-for="entry in group.items" :key="entry.slug">
              <a :href="entry.link?.url ?? `${base}${entry.slug}/`" class="catalog-row">
                <span class="row-main">
                  <span class="row-name">
                    <code v-if="!isProseName(entry.name)">{{ entry.name }}</code
                    ><span v-else class="row-name-text" v-html="inline(entry.name)"></span>
                    <span v-if="entry.skathiOnly" class="flag flag-skathi">Skathi</span>
                    <span v-if="entry.experimental" class="flag flag-experimental">Experimental</span>
                    <span v-if="entry.link" class="flag flag-link">{{ entry.link.title }}</span>
                    <span v-if="entry.fresh" class="flag flag-new" :title="entry.fresh">{{ newLabel }}</span>
                  </span>
                  <span class="row-summary" v-html="inline(entry.summary)"></span>
                </span>
                <PlatformMatrix
                  v-if="entry.split && platform === 'all'"
                  :platforms="entry.platforms"
                  :since="entry.platformSince"
                />
                <span v-else class="row-status" :data-status="statusOf(entry)"
                  ><StatusMark :status="statusOf(entry)" :size="14" />{{ statusLabels[statusOf(entry)] }}</span
                >
                <span class="row-since" :class="{ 'is-new': isNew(entry.changed) }">{{ entry.since ?? "" }}</span>
              </a>
            </li>
          </ul>
        </section>
      </template>
      <div v-if="!visible.length" class="catalog-empty">
        <svg
          width="28"
          height="28"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.4"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="9" stroke-dasharray="2.4 3.2" />
          <path d="M9.5 9.5l5 5M14.5 9.5l-5 5" />
        </svg>
        <p>
          <strong
            >Nothing matches<template v-if="query.trim()"> “{{ query.trim() }}”</template>.</strong
          >
          What this list does not have, Skathi does not support. In
          <code>sk dev</code> it prints an advisory where it drops one.
        </p>
        <button type="button" class="link-button" @click="reset">Clear search and filters</button>
      </div>
    </div>
  </section>
</template>
