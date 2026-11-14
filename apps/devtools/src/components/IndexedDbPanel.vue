<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { byteLabel } from "../inspector-model";
import type {
  DatabaseList,
  DatabaseRecord,
  DatabaseRecords,
  DatabaseSchema,
  InspectorRequest,
  Preview,
} from "../inspector-model";
const props = defineProps<{
  height: number;
  request?: InspectorRequest | undefined;
  sessionKey?: string | number | undefined;
}>();
const catalog = ref<DatabaseList | null>(null),
  schema = ref<DatabaseSchema | null>(null),
  page = ref<DatabaseRecords | null>(null);
const database = ref(""),
  store = ref(""),
  index = ref(""),
  search = ref(""),
  busy = ref(false),
  error = ref("");
const cursors = ref<(string | null)[]>([null]),
  pageIndex = ref(0),
  selected = ref<DatabaseRecord | null>(null);
const activeStore = computed(() => schema.value?.stores.find((value) => value.name === store.value));
let epoch = 0;
function text(value: Preview | undefined) {
  return value?.preview ?? "—";
}
function keyPath(value: unknown) {
  return typeof value === "string" ? value : value == null ? "out-of-line keys" : JSON.stringify(value);
}
async function run(task: (token: number, request: InspectorRequest) => Promise<void>) {
  if (!props.request) return;
  const token = ++epoch,
    request = props.request;
  busy.value = true;
  error.value = "";
  try {
    await task(token, request);
  } catch (reason) {
    if (token === epoch) error.value = String(reason);
  } finally {
    if (token === epoch) busy.value = false;
  }
}
async function fetchRecords(token: number, request: InspectorRequest) {
  if (!database.value || !store.value) {
    page.value = null;
    return;
  }
  page.value = null;
  selected.value = null;
  const value = await request<DatabaseRecords>({
    cmd: "indexeddb",
    op: "records",
    database: database.value,
    store: store.value,
    ...(index.value ? { index: index.value } : {}),
    after: cursors.value[pageIndex.value] || undefined,
    limit: 50,
    search: search.value,
  });
  if (token === epoch) {
    page.value = value;
    selected.value = null;
  }
}
async function fetchSchema(token: number, request: InspectorRequest) {
  const value = await request<DatabaseSchema>({ cmd: "indexeddb", op: "schema", database: database.value });
  if (token !== epoch) return;
  schema.value = value;
  store.value = value.stores.some((value) => value.name === store.value) ? store.value : value.stores[0]?.name || "";
  index.value = "";
  cursors.value = [null];
  pageIndex.value = 0;
  await fetchRecords(token, request);
}
function refresh() {
  void run(async (token, request) => {
    const value = await request<DatabaseList>({ cmd: "indexeddb", op: "list" });
    if (token !== epoch) return;
    catalog.value = value;
    database.value = value.databases.some((value) => value.name === database.value)
      ? database.value
      : value.databases[0]?.name || "";
    if (database.value) await fetchSchema(token, request);
    else {
      schema.value = null;
      page.value = null;
    }
  });
}
function chooseDatabase() {
  schema.value = null;
  page.value = null;
  selected.value = null;
  void run(fetchSchema);
}
function resetPage() {
  cursors.value = [null];
  pageIndex.value = 0;
  void run(fetchRecords);
}
function chooseStore() {
  index.value = "";
  resetPage();
}
function next() {
  if (!page.value?.nextCursor) return;
  cursors.value = [...cursors.value.slice(0, pageIndex.value + 1), page.value.nextCursor];
  pageIndex.value++;
  void run(fetchRecords);
}
function previous() {
  if (!pageIndex.value) return;
  pageIndex.value--;
  void run(fetchRecords);
}
watch(
  [() => props.request, () => props.sessionKey],
  () => {
    epoch++;
    busy.value = false;
    catalog.value = null;
    schema.value = null;
    page.value = null;
    selected.value = null;
    database.value = "";
    store.value = "";
    index.value = "";
    error.value = "";
    search.value = "";
    if (props.request) refresh();
  },
  { immediate: true },
);
</script>

<template>
  <section class="indexed" :style="{ height: height + 'px' }" aria-label="IndexedDB inspector">
    <div class="database-toolbar">
      <button class="btn" :disabled="!request || busy" @click="refresh">
        {{ busy ? "Loading…" : "Refresh databases" }}</button
      ><span class="muted">Read-only inspection</span
      ><span v-if="catalog" class="backend mono">{{ catalog.backend }}</span>
    </div>
    <div v-if="error" class="feedback failure" role="alert">
      {{ error }}<button class="btn" :disabled="busy" @click="refresh">Retry</button>
    </div>
    <div v-if="!request" class="feedback">Attach to a target to inspect its databases.</div>
    <div v-else-if="catalog && !catalog.available" class="feedback">IndexedDB is unavailable in this target build.</div>
    <div v-else-if="catalog && !catalog.databases.length" class="feedback">
      No IndexedDB databases are open or stored for this target. Refresh after the application creates one.
    </div>
    <template v-if="catalog?.available && catalog.databases.length">
      <div class="selectors">
        <label
          >Database<select v-model="database" class="input" :disabled="busy" @change="chooseDatabase">
            <option v-for="value in catalog.databases" :key="value.name" :value="value.name">
              {{ value.name }} · v{{ value.version }}
            </option>
          </select></label
        ><label
          >Object store<select
            v-model="store"
            class="input"
            :disabled="busy || !schema?.stores.length"
            @change="chooseStore"
          >
            <option v-for="value in schema?.stores || []" :key="value.name" :value="value.name">
              {{ value.name }} · {{ value.count }} records
            </option>
          </select></label
        ><label
          >Index<select v-model="index" class="input" :disabled="busy || !activeStore" @change="resetPage">
            <option value="">Primary key</option>
            <option v-for="value in activeStore?.indexes || []" :key="value.name" :value="value.name">
              {{ value.name }}{{ value.unique ? " · unique" : "" }}{{ value.multiEntry ? " · multi-entry" : "" }}
            </option>
          </select></label
        >
      </div>
      <div v-if="activeStore" class="schema-line">
        <span
          >Key path: <b class="mono">{{ keyPath(activeStore.keyPath) }}</b></span
        ><span>{{ activeStore.autoIncrement ? "Auto-increment keys" : "Explicit keys" }}</span
        ><span>{{ activeStore.indexes.length }} indexes</span>
      </div>
      <form class="search-row" @submit.prevent="resetPage">
        <input
          v-model="search"
          class="input"
          placeholder="Search captured key and value previews"
          aria-label="Search IndexedDB previews"
          maxlength="256"
          :disabled="busy"
        /><button class="btn" :disabled="busy || !store">Search</button><span class="muted">50 records per page</span>
      </form>
      <div v-if="schema && !schema.stores.length" class="feedback">This database has no object stores.</div>
      <div class="record-head">
        <span class="key">Key</span><span class="value">Value preview</span><span class="type">Type</span
        ><span class="size">Encoded</span>
      </div>
      <div class="records" :style="{ height: Math.max(80, height - (selected ? 330 : 210)) + 'px' }" :aria-busy="busy">
        <button
          v-for="(record, position) in page?.records || []"
          :key="position"
          class="record"
          :class="{ selected: selected === record }"
          @click="selected = record"
        >
          <span class="key mono" :title="text(record.key)">{{ text(record.key) }}</span
          ><span class="value mono" :title="text(record.value)">{{ text(record.value) }}</span
          ><span class="type">{{ record.value.type }}</span
          ><span class="size mono">{{ byteLabel(record.encodedBytes) }}</span>
        </button>
        <div v-if="!busy && page && !page.records.length" class="feedback">
          {{
            search
              ? "No matching previews on this scan. Clear the search or continue to the next page if more records remain."
              : "This object store has no records."
          }}
        </div>
      </div>
      <div class="pagination">
        <button class="btn" :disabled="busy || !pageIndex" @click="previous">Previous page</button
        ><span
          >Page {{ pageIndex + 1 }} · {{ page?.records.length || 0 }} records · scanned {{ page?.scanned || 0 }}</span
        ><button class="btn" :disabled="busy || !page?.hasMore || !page.nextCursor" @click="next">Next page</button
        ><span v-if="page?.truncated" class="warning">Preview / scan limit reached</span>
      </div>
      <div v-if="selected" class="record-detail">
        <header>
          <strong>{{ selected.value.type }}</strong
          ><span class="mono">{{ text(selected.primaryKey) }}</span
          ><span v-if="selected.value.size !== undefined">{{ byteLabel(selected.value.size) }}</span
          ><span v-if="selected.value.name">{{ selected.value.name }}</span
          ><span v-if="selected.value.mime">{{ selected.value.mime }}</span
          ><span v-if="selected.value.ref !== undefined">Reference #{{ selected.value.ref }}</span
          ><span v-if="selected.value.truncated" class="warning">Truncated preview</span
          ><button class="btn" @click="selected = null">Close detail</button>
        </header>
        <pre>{{ selected.value.preview }}</pre>
        <div
          v-for="(child, position) in (selected.value.children || []).slice(0, 50)"
          :key="position"
          class="preview-field"
        >
          <span class="mono">{{ child.name }}</span
          ><span class="muted">{{ child.value.type }}</span
          ><span class="mono">{{ child.value.preview }}</span>
        </div>
      </div>
    </template>
    <div v-else-if="busy && !catalog" class="feedback" role="status">Reading database metadata…</div>
  </section>
</template>

<style scoped>
.indexed {
  min-width: 0;
  overflow-y: auto;
  background: var(--surface-panel);
  color: var(--text);
  font-size: var(--fs-s);
}
button {
  font-family: inherit;
  cursor: pointer;
}
button:disabled,
select:disabled,
input:disabled {
  opacity: 0.45;
  cursor: default;
}
button:focus-visible,
select:focus-visible {
  outline: 2px solid var(--line-focus);
  outline-offset: -2px;
}
.database-toolbar,
.schema-line,
.pagination,
.search-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-4);
  padding: var(--sp-3) var(--sp-4);
  border-bottom: 1px solid var(--line);
  min-height: 24px;
}
.backend {
  margin-left: auto;
}
.muted {
  color: var(--text-muted);
}
.feedback {
  padding: var(--sp-6) var(--sp-5);
  line-height: 1.5;
  color: var(--text-muted);
}
.failure {
  color: var(--bad);
  display: flex;
  gap: var(--sp-4);
  align-items: center;
}
.warning {
  color: var(--warn);
}
.selectors {
  display: flex;
  gap: var(--sp-5);
  padding: var(--sp-4);
  flex-wrap: wrap;
  background: var(--surface-panel);
}
.selectors label {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  flex: 1;
  min-width: 140px;
  color: var(--text-muted);
}
.selectors select {
  width: 100%;
  height: 25px;
}
.schema-line {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.schema-line b {
  font-weight: 400;
  color: var(--text);
}
.search-row input {
  flex: 1;
  min-width: 140px;
}
.record-head,
.record {
  display: flex;
  align-items: center;
  gap: var(--sp-4);
  box-sizing: border-box;
  padding: 0 var(--sp-4);
  width: 100%;
  min-width: 0;
}
.record-head {
  height: 24px;
  background: var(--surface-raised);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.records {
  overflow-y: auto;
}
.record {
  height: var(--h-bar);
  background: var(--surface-panel);
  color: var(--text);
  border: 0;
  border-bottom: 1px solid var(--line-subtle);
  font-size: var(--fs-s);
  text-align: left;
}
.record:hover {
  background: var(--surface-hover);
}
.record.selected {
  background: var(--surface-active);
}
.key {
  width: 24%;
  flex-shrink: 0;
  color: var(--info);
}
.value {
  flex: 1;
  min-width: 0;
  color: var(--text-strong);
}
.type {
  width: 72px;
  flex-shrink: 0;
  color: var(--text-muted);
}
.size {
  width: 68px;
  flex-shrink: 0;
  text-align: right;
  color: var(--text-muted);
}
.key,
.value,
.type {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pagination {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
.record-detail {
  padding: var(--sp-4) var(--sp-4);
  background: var(--surface-base);
  border-top: 1px solid var(--line);
  max-height: 240px;
  overflow-y: auto;
}
.record-detail header {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-4);
}
.record-detail header .btn {
  margin-left: auto;
}
.record-detail pre {
  color: var(--text-strong);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: var(--fs-s);
  line-height: 1.5;
  margin: var(--sp-4) 0;
}
.preview-field {
  display: flex;
  gap: var(--sp-4);
  padding: var(--sp-1) 0;
  line-height: 1.4;
}
.preview-field > :first-child {
  min-width: 80px;
  color: var(--info);
}
.preview-field > :last-child {
  overflow-wrap: anywhere;
}
@media (max-width: 600px) {
  .type {
    width: 50px;
  }
  .size {
    display: none;
  }
  .selectors {
    gap: var(--sp-3);
  }
  .selectors label {
    min-width: 120px;
  }
}
</style>
