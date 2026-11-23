import type { Details } from "../../details";

export const details: Details = {
  "readable-stream": {
    examples: [
      {
        title: "Stream search results page by page",
        lang: "ts",
        code: `async function* pages(query: string) {
  for (let page = 1; ; page++) {
    const res = await fetch("https://api.example.com/v1/search?q=" + encodeURIComponent(query) + "&page=" + page);
    const batch: unknown[] = await res.json();
    if (batch.length === 0) return;
    yield* batch;
  }
}

for await (const item of ReadableStream.from(pages("invoices"))) {
  addRow(item);
}`,
      },
      {
        title: "Wrap a timer as a stream you can cancel",
        lang: "ts",
        code: `function ticks(intervalMs: number): ReadableStream<number> {
  let timer = 0;
  return new ReadableStream({
    start(controller) {
      timer = setInterval(() => controller.enqueue(Date.now()), intervalMs);
    },
    cancel() {
      clearInterval(timer);
    },
  });
}

const reader = ticks(1000).getReader();
const { value } = await reader.read();
await reader.cancel();`,
      },
    ],
    edgeCases: [
      "A negative or `NaN` `highWaterMark` throws a `RangeError`. The default is 1.",
      "`enqueue` after `close()` throws a `TypeError`, and so does a second `getReader()` while the first reader holds the lock.",
      "Leaving a `for await` loop early cancels the stream. Iterate `stream.values({ preventCancel: true })` to keep it open.",
      "`blob.stream()` and `file.stream()` deliver 64 KiB chunks.",
    ],
  },
  "writable-stream": {
    examples: [
      {
        title: "Save a compressed export to a file the user picks",
        lang: "ts",
        code: `exportButton.addEventListener("click", async () => {
  const handle = await showSaveFilePicker({ suggestedName: "session.log.gz" });
  const file = await handle.createWritable();
  await sessionLog.stream().pipeThrough(new CompressionStream("gzip")).pipeTo(file);
});`,
        note: "The file's writable is a `WritableStream`, so a pipe can end in it; `pipeTo` closes it, which saves the file.",
      },
      {
        title: "Send telemetry through a sink with a bounded queue",
        lang: "ts",
        code: `const telemetry = new WritableStream<string>(
  {
    async write(line) {
      await fetch("https://telemetry.example.com/ingest", { method: "POST", body: line });
    },
  },
  new CountQueuingStrategy({ highWaterMark: 8 }),
);
const writer = telemetry.getWriter();

async function track(event: string) {
  await writer.ready;
  writer.write(JSON.stringify({ event, at: Date.now() }));
}`,
      },
    ],
    edgeCases: [
      "`write()` after `close()` rejects with a `TypeError`. So do `write()`, `close()` and `abort()` on a writer after `releaseLock()`.",
      "`abort()` and `close()` on a locked stream reject with a `TypeError`: call them on the writer.",
      "A `size()` that returns a negative, `NaN` or infinite number errors the stream with a `RangeError`.",
    ],
  },
  "transform-stream": {
    examples: [
      {
        title: "Read a picked file line by line",
        lang: "ts",
        code: `function lines(): TransformStream<Uint8Array, string> {
  const decoder = new TextDecoder();
  let rest = new Uint8Array(0);
  return new TransformStream({
    transform(chunk, controller) {
      const bytes = new Uint8Array(rest.length + chunk.length);
      bytes.set(rest);
      bytes.set(chunk, rest.length);
      let start = 0;
      for (let i = bytes.indexOf(10); i !== -1; i = bytes.indexOf(10, start)) {
        controller.enqueue(decoder.decode(bytes.subarray(start, i)));
        start = i + 1;
      }
      rest = bytes.slice(start);
    },
    flush(controller) {
      if (rest.length) controller.enqueue(decoder.decode(rest));
    },
  });
}

for await (const line of file.stream().pipeThrough(lines())) importRow(line);`,
        note: "It splits on the newline byte before decoding, so a character cut between two chunks is never decoded in halves.",
      },
      {
        title: "Turn records into NDJSON",
        lang: "ts",
        code: `const toNdjson = new TransformStream<object, string>({
  transform(record, controller) {
    controller.enqueue(JSON.stringify(record) + "\\n");
  },
});

const ndjson = ReadableStream.from(records).pipeThrough(toNdjson);`,
      },
    ],
    edgeCases: [
      "Without a `transform` function, chunks pass through unchanged.",
      "An exception in `transform` errors both sides: the pending `write()` rejects, and the readable side errors with the same value.",
      "If `flush` throws, the readable side errors first, then the writer's `close()` rejects.",
      "`controller.terminate()` closes the readable side and errors the writable side with a `TypeError`.",
    ],
  },
  "byte-length-queuing-strategy": {
    examples: [
      {
        title: "Upload a file in parts with up to 4 MiB in flight",
        lang: "ts",
        code: `const parts = new WritableStream<Uint8Array>(
  { write: (chunk) => uploadPart(chunk) },
  new ByteLengthQueuingStrategy({ highWaterMark: 4 * 1024 * 1024 }),
);
const writer = parts.getWriter();
for await (const chunk of file.stream()) {
  await writer.ready;
  writer.write(chunk);
}
await writer.close();`,
      },
    ],
    edgeCases: [
      "`highWaterMark` is required: `new ByteLengthQueuingStrategy({})` throws a `TypeError`.",
      "A chunk without `byteLength`, such as a string, counts as 1 instead of erroring the stream.",
      "A chunk handed straight to a waiting reader is not measured: `size` runs only for chunks that are queued.",
    ],
  },
  "count-queuing-strategy": {
    examples: [
      {
        title: "Warn when a slow consumer lets messages pile up",
        lang: "ts",
        code: `const inbox = new ReadableStream(
  {
    start(controller) {
      socket.onmessage = (e) => {
        controller.enqueue(JSON.parse(e.data));
        if (controller.desiredSize! <= 0) console.warn("Inbox is backing up");
      };
    },
  },
  new CountQueuingStrategy({ highWaterMark: 10 }),
);`,
      },
    ],
    edgeCases: [
      "`highWaterMark` is required: `new CountQueuingStrategy({})` throws a `TypeError`.",
      "A chunk handed straight to a waiting reader is not counted.",
    ],
  },
  "compression-stream": {
    examples: [
      {
        title: "Gzip a backup before you save it",
        lang: "ts",
        code: `async function gzip(data: Blob): Promise<Blob> {
  const chunks: Uint8Array[] = [];
  for await (const chunk of data.stream().pipeThrough(new CompressionStream("gzip"))) {
    chunks.push(chunk);
  }
  return new Blob(chunks, { type: "application/gzip" });
}

const backup = await gzip(new Blob([JSON.stringify(database)]));`,
        note: "Collect the chunks yourself: `new Response(stream)` does not accept a stream here.",
      },
    ],
    edgeCases: [
      'The formats are `"gzip"`, `"deflate"` and `"deflate-raw"`, matched case-sensitively: `"GZIP"` or `"br"` throws a `TypeError`.',
      "Chunks must be an `ArrayBuffer` or a typed array. A string chunk errors the stream with a `TypeError`; encode it with `TextEncoder` first.",
      "Small writes often produce no output until the writable side closes, and empty chunks are never enqueued.",
    ],
  },
  "decompression-stream": {
    examples: [
      {
        title: "Open a gzipped JSON file the user picked",
        lang: "ts",
        code: `async function readGzipJson(file: File) {
  const chunks: Uint8Array[] = [];
  for await (const chunk of file.stream().pipeThrough(new DecompressionStream("gzip"))) {
    chunks.push(chunk);
  }
  return JSON.parse(await new Blob(chunks).text());
}`,
      },
    ],
    edgeCases: [
      "Truncated input errors the stream with a `TypeError` when the writable side closes. It never yields a short result.",
      "A gzip stream whose checksum or length does not match, or whose header is not gzip, errors the stream with a `TypeError`.",
    ],
  },
  "text-encoder": {
    examples: [
      {
        title: "Check a message's size in bytes before sending it",
        lang: "ts",
        code: `const encoder = new TextEncoder();

function fitsInFrame(text: string): boolean {
  return encoder.encode(text).byteLength <= 4096;
}`,
      },
      {
        title: "Write a length-prefixed string into a fixed buffer",
        lang: "ts",
        code: `const frame = new Uint8Array(256);
const { read, written } = new TextEncoder().encodeInto(title, frame.subarray(4));
new DataView(frame.buffer).setUint32(0, written);
if (read < title.length) console.warn("Title cut to fit the frame");`,
      },
    ],
    edgeCases: [
      "`encodeInto` writes whole characters only: a character that does not fit is left out, and `read` counts UTF-16 code units.",
      "`encodeInto` throws a `TypeError` unless the destination is a `Uint8Array`.",
      "`encode()` with no argument returns an empty `Uint8Array`.",
    ],
  },
  "text-decoder": {
    examples: [
      {
        title: "Import a legacy Windows-1251 export",
        lang: "ts",
        code: `async function readLegacyCsv(file: File): Promise<string[][]> {
  const text = new TextDecoder("windows-1251").decode(await file.arrayBuffer());
  return text.split(/\\r?\\n/).map((line) => line.split(";"));
}`,
      },
      {
        title: "Reject a file that is not valid UTF-8",
        lang: "ts",
        code: `const strict = new TextDecoder("utf-8", { fatal: true });
try {
  notes.value = strict.decode(await file.arrayBuffer());
} catch {
  showError(file.name + " is not a UTF-8 text file");
}`,
      },
    ],
    edgeCases: [
      'An unknown label throws a `RangeError`. `encoding` reports the canonical name, such as `"windows-1251"` for `"cp1251"`.',
      "A leading BOM of the decoder's own encoding is removed unless `ignoreBOM` is `true`. A BOM never switches the encoding.",
      "With `fatal: true`, invalid input throws a `TypeError`.",
      "With `{ stream: true }` a character cut between two chunks is held until the next call, in every encoding. A call without it, or `decode()` with no input, flushes: a dangling partial character becomes U+FFFD, or a `TypeError` with `fatal`.",
    ],
  },
  atob: {
    examples: [
      {
        title: "Decode a Base64 payload into bytes",
        lang: "ts",
        code: `function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}`,
      },
      {
        title: "Read the claims of a JWT",
        lang: "ts",
        code: `function jwtClaims(token: string) {
  const part = token.split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
  return JSON.parse(new TextDecoder().decode(base64ToBytes(part)));
}`,
        note: "A JWT uses the URL-safe alphabet, which `atob` does not accept.",
      },
    ],
    edgeCases: [
      "ASCII whitespace is skipped, padding is optional, and `=` is ignored wherever it appears.",
      "`-` and `_` throw `InvalidCharacterError`: map them to `+` and `/` first.",
      "The result has one character per byte. Decode UTF-8 text by passing the bytes to `TextDecoder`.",
    ],
  },
  btoa: {
    examples: [
      {
        title: "Build a Basic authorization header",
        lang: "ts",
        code: `function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

const auth = "Basic " + bytesToBase64(new TextEncoder().encode(user + ":" + password));`,
        note: "Encoding to UTF-8 first keeps names with non-Latin characters working.",
      },
    ],
    edgeCases: ["A character above U+00FF throws `InvalidCharacterError`."],
  },
  blob: {
    examples: [
      {
        title: "Check a file's type from its first bytes",
        lang: "ts",
        code: `async function isPng(file: File): Promise<boolean> {
  const head = await file.slice(0, 8).bytes();
  return head[0] === 0x89 && head[1] === 0x50 && head[2] === 0x4e && head[3] === 0x47;
}`,
        note: "Only the sliced bytes are read from disk.",
      },
      {
        title: "Offer generated CSV as a download",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, onUnmounted, watch } from "vue";

const props = defineProps<{ rows: string[][] }>();
const href = computed(() => {
  const csv = props.rows.map((row) => row.join(",")).join("\\r\\n");
  return URL.createObjectURL(new Blob([csv], { type: "text/csv" }));
});
watch(href, (_, old) => URL.revokeObjectURL(old));
onUnmounted(() => URL.revokeObjectURL(href.value));
</script>

<template>
  <a :href="href" download="report.csv">Export CSV</a>
</template>`,
      },
    ],
    edgeCases: [
      '`type` is lowercased, and a type with a character outside U+0020–U+007E becomes `""`.',
      "`slice()` never copies, and a slice of a picked file reads nothing until you call `text()`, `arrayBuffer()`, `bytes()` or `stream()`.",
      "Reading a picked file that changed on disk after it was picked rejects with `NotReadableError`.",
      '`endings: "native"` turns every CR, LF and CRLF in string parts into the platform\'s line break. Any value but `"native"` or `"transparent"` throws a `TypeError`.',
      "A file-backed Blob used as a `fetch` body, in `FormData` or in `WebSocket.send` is read on the UI thread; past 16 MiB an advisory names `stream()`.",
    ],
  },
  file: {
    examples: [
      {
        title: "Name a generated report before sharing it",
        lang: "ts",
        code: `shareButton.addEventListener("click", async () => {
  const report = new File([csv], "report-2026-11.csv", { type: "text/csv" });
  if (navigator.canShare({ files: [report] })) {
    await navigator.share({ files: [report], title: "Monthly report" });
  }
});`,
      },
    ],
    edgeCases: [
      "`new File(bits)` without a name throws a `TypeError`.",
      "`lastModified` is floored to whole milliseconds, `NaN` becomes 0, and the default is the time of construction.",
      '`new File([picked], "renamed.txt")` stays backed by the file on disk and is read only when needed.',
    ],
  },
  "file-list": {
    examples: [
      {
        title: "Import several CSV files at once",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const imported = ref<string[]>([]);

async function onPick(e: Event) {
  const files = (e.target as HTMLInputElement).files!;
  for (const file of files) {
    const text = await file.text();
    imported.value.push(file.name + ": " + text.split("\\n").length + " lines");
  }
}
</script>

<template>
  <input type="file" accept=".csv" multiple @change="onPick" />
  <ul><li v-for="line in imported" :key="line">{{ line }}</li></ul>
</template>`,
      },
    ],
    edgeCases: [
      "A cancelled picker fires `cancel` and keeps the previous `files`.",
      'Setting `input.value = ""` clears `files`; any other value throws `InvalidStateError`.',
      "`input.value` reads `C:\\fakepath\\` followed by the first file's name, on every platform.",
      "Assigning `input.files` anything but a `FileList` throws a `TypeError`, and assigning one fires no event.",
      "`dataTransfer.files` is empty during `dragenter` and `dragover`; the files are readable in `drop`.",
    ],
  },
  "file-reader": {
    examples: [
      {
        title: "Preview a picked image",
        lang: "ts",
        code: `function preview(file: File) {
  const reader = new FileReader();
  reader.onload = () => (thumbnail.value = reader.result as string);
  reader.onerror = () => console.error("Could not read " + file.name, reader.error);
  reader.readAsDataURL(file);
}`,
      },
      {
        title: "Read text in a legacy encoding",
        lang: "ts",
        code: `const reader = new FileReader();
reader.onload = () => importSubtitles(reader.result as string);
reader.readAsText(file, "shift_jis");`,
      },
    ],
    edgeCases: [
      "Starting a read while one is running throws `InvalidStateError`.",
      "Events fire as `loadstart`, `progress`, then `load` or `error`, then `loadend`. A new read started in `onload` suppresses the first read's `loadend`.",
      "`readAsDataURL` of a Blob with no type produces `data:application/octet-stream;base64,...`.",
    ],
  },
  "form-data": {
    examples: [
      {
        title: "Upload a post with an attachment",
        lang: "ts",
        code: `const form = new FormData();
form.append("title", title);
form.append("attachment", file);
form.append("thumbnail", thumbnailBlob, "thumb.png");
const res = await fetch("https://api.example.com/v1/posts", { method: "POST", body: form });`,
      },
    ],
    edgeCases: [
      "As a body it sets `multipart/form-data` with its boundary. A `Content-Type` you set yourself is kept as is, without the boundary.",
      'A `Blob` value becomes a `File` named `"blob"`, and a third argument renames it. Only a `File` appended without a name comes back from `get()` as the same object.',
      "A file name with a value that is not a `Blob` throws a `TypeError`.",
    ],
  },
  "abort-controller": {
    examples: [
      {
        title: "Cancel a request when the view closes",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const stats = ref<Record<string, number> | null>(null);
const controller = new AbortController();

onMounted(async () => {
  try {
    const res = await fetch("https://api.example.com/v1/stats", { signal: controller.signal });
    stats.value = await res.json();
  } catch (e) {
    if ((e as DOMException).name !== "AbortError") throw e;
  }
});
onUnmounted(() => controller.abort());
</script>`,
      },
      {
        title: "Remove a group of listeners at once",
        lang: "ts",
        code: `const editorListeners = new AbortController();
window.addEventListener("keydown", onShortcut, { signal: editorListeners.signal });
window.addEventListener("resize", relayout, { signal: editorListeners.signal });

function closeEditor() {
  editorListeners.abort();
}`,
      },
    ],
    edgeCases: [
      "`abort()` with no argument sets `reason` to an `AbortError` DOMException. A second `abort()` changes nothing.",
      "Listeners added with `{ signal }` are removed before the `abort` event fires.",
      "Aborting a `fetch` in flight closes its socket at once.",
    ],
  },
  "abort-signal": {
    examples: [
      {
        title: "Give an export a deadline and a cancel button",
        lang: "ts",
        code: `const cancel = new AbortController();
cancelButton.onclick = () => cancel.abort();
const signal = AbortSignal.any([cancel.signal, AbortSignal.timeout(15_000)]);
try {
  await fetch(exportUrl, { signal });
} catch (e) {
  if ((e as DOMException).name === "TimeoutError") showToast("The export took too long");
}`,
      },
      {
        title: "Stop a long loop between steps",
        lang: "ts",
        code: `async function indexFiles(files: File[], signal: AbortSignal) {
  for (const file of files) {
    signal.throwIfAborted();
    await indexOne(file);
  }
}`,
      },
    ],
    edgeCases: [
      "`AbortSignal.timeout(ms)` aborts with a `TimeoutError` DOMException. A negative delay counts as 0.",
      "`AbortSignal.any` returns an already aborted signal when an input is aborted, with that input's reason. `AbortSignal.any([])` never aborts.",
      "Once a combined signal has aborted, later aborts of its other inputs do not change its `reason`.",
      "Passing anything but `AbortSignal`s to `AbortSignal.any` throws a `TypeError`.",
    ],
  },
  "crypto-get-random-values": {
    examples: [
      {
        title: "Generate an invite code",
        lang: "ts",
        code: `const ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

function inviteCode(length = 8): string {
  const bytes = crypto.getRandomValues(new Uint8Array(length));
  return Array.from(bytes, (b) => ALPHABET[b % ALPHABET.length]).join("");
}`,
        note: "The alphabet has 32 letters, which divides 256, so every letter is equally likely.",
      },
    ],
    edgeCases: [
      "More than 65,536 bytes throws `QuotaExceededError`: fill a larger buffer in slices.",
      "A float array or a `DataView` throws `TypeMismatchError`.",
      "It returns the array it filled.",
    ],
  },
  "crypto-random-uuid": {
    examples: [
      {
        title: "Give a new draft a stable ID",
        lang: "ts",
        code: `function newDraft(title = "") {
  return {
    id: crypto.randomUUID(),
    title,
    createdAt: Date.now(),
  };
}`,
      },
    ],
    edgeCases: [
      "IDs are lowercase version 4 UUIDs: the 13th digit is `4`, the 17th is one of `8`, `9`, `a`, `b`.",
      "Calling it detached, as in `const f = crypto.randomUUID; f()`, throws a `TypeError`.",
    ],
  },
  "subtle-crypto": {
    examples: [
      {
        title: "Hash a file to find duplicates",
        lang: "ts",
        code: `async function sha256Hex(data: BufferSource): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

const id = await sha256Hex(await file.arrayBuffer());`,
      },
      {
        title: "Build a cache key from a query",
        lang: "ts",
        code: `async function cacheKey(query: object): Promise<string> {
  const bytes = new TextEncoder().encode(JSON.stringify(query));
  return "q:" + (await sha256Hex(bytes));
}`,
      },
    ],
    edgeCases: [
      "Algorithm names are case-insensitive and may be a string or `{ name }`. Any other algorithm rejects with `NotSupportedError`.",
      "A string as `data` rejects with a `TypeError`: encode it with `TextEncoder` first.",
    ],
  },
};
