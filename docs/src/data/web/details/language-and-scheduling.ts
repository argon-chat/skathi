import type { Details } from "../../details";

export const details: Details = {
  "set-timeout": {
    examples: [
      {
        title: "Hide a toast after a few seconds",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";

const message = ref("");
let hideTimer = 0;

function showToast(text: string) {
  message.value = text;
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => (message.value = ""), 4000);
}

onBeforeUnmount(() => clearTimeout(hideTimer));
defineExpose({ showToast });
</script>

<template>
  <div v-if="message" class="toast">{{ message }}</div>
</template>`,
        note: "Clearing the previous timer first restarts the countdown when a second toast arrives.",
      },
      {
        title: "Debounce a search field",
        lang: "ts",
        code: `let pending = 0;

export function onSearchInput(query: string, run: (q: string) => void) {
  clearTimeout(pending);
  pending = setTimeout(run, 250, query.trim());
}`,
        note: "Arguments after the delay are passed to the callback.",
      },
    ],
    edgeCases: [
      'A delay that is not a number, such as the string `"250"`, counts as `0`; a browser converts it to a number first. A negative delay also counts as `0`.',
      "A timeout scheduled from inside a timer callback never runs in the same pump, even with a delay of `0`; it waits for the next one.",
      "Clearing a timer from an earlier callback in the same pump stops it, even when it was already due.",
      "While the window is minimized or hidden, timers keep running on a background tick of 100 ms, so a due timer fires at most about 100 ms late.",
      "An exception thrown in the callback goes to the host's error log (`[js uncaught]`); it does not dispatch `error` at the window.",
    ],
  },
  "set-interval": {
    examples: [
      {
        title: "Show a call's elapsed time",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

const startedAt = Date.now();
const seconds = ref(0);
let ticker = 0;

onMounted(() => {
  ticker = setInterval(() => (seconds.value = Math.floor((Date.now() - startedAt) / 1000)), 1000);
});
onBeforeUnmount(() => clearInterval(ticker));

const label = computed(() => Math.floor(seconds.value / 60) + ":" + String(seconds.value % 60).padStart(2, "0"));
</script>

<template>
  <span class="call-timer">{{ label }}</span>
</template>`,
        note: "The label is computed from a start time, so a late tick never makes the clock drift.",
      },
      {
        title: "Poll a sync endpoint until it reports done",
        lang: "ts",
        code: `export function waitForSync(jobId: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const poll = setInterval(async () => {
      const res = await fetch("https://api.example.com/sync/" + jobId);
      const { state } = res.ok ? await res.json() : { state: "failed" };
      if (state === "pending") return;
      clearInterval(poll);
      if (state === "done") resolve();
      else reject(new Error("sync " + jobId + " " + state));
    }, 2000);
  });
}`,
      },
    ],
    edgeCases: [
      "Each interval fires at most once per pump and never fires twice to catch up on missed ticks; the next run is scheduled from the time the pump started plus the interval.",
      "A delay below 1 ms, including `0`, is raised to 1 ms.",
      "`clearInterval` inside the interval's own callback stops it; the interval is not rescheduled afterwards.",
      "`clearTimeout` and `clearInterval` both clear either kind of timer: timeouts and intervals share one id space, as in a browser.",
    ],
  },
  "queue-microtask": {
    examples: [
      {
        title: "Batch several state changes into one save",
        lang: "ts",
        code: `const dirty = new Set<string>();
let scheduled = false;

export function markDirty(noteId: string, save: (ids: string[]) => void) {
  dirty.add(noteId);
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => {
    scheduled = false;
    save([...dirty]);
    dirty.clear();
  });
}`,
        note: "Every `markDirty` call in the same task ends up in one `save` call.",
      },
    ],
    edgeCases: [
      "A value that is not callable throws a `TypeError` (`queueMicrotask: argument must be callable`).",
      "Timers and `requestAnimationFrame` callbacks that run in the same pump share one microtask checkpoint at the end of the pump: two timers due together both run before a microtask the first one queued. A browser runs that microtask between them.",
      "Each input event the host dispatches gets its own checkpoint, so microtasks queued by a `keydown` listener run before the `keyup` listeners.",
    ],
  },
  "request-animation-frame": {
    examples: [
      {
        title: "Animate a progress ring from script",
        lang: "ts",
        code: `export function animateRing(ring: HTMLElement, to: number, duration = 400) {
  const from = Number(ring.dataset.value ?? 0);
  let start = 0;
  const step = (now: number) => {
    start ||= now;
    const t = Math.min((now - start) / duration, 1);
    const value = from + (to - from) * (1 - (1 - t) ** 3);
    ring.style.setProperty("--progress", String(value));
    if (t < 1) requestAnimationFrame(step);
    else ring.dataset.value = String(to);
  };
  requestAnimationFrame(step);
}`,
      },
      {
        title: "Coalesce pointer moves into one update per frame",
        lang: "ts",
        code: `let frame = 0;
let lastX = 0;

export function onSplitterMove(e: PointerEvent, apply: (x: number) => void) {
  lastX = e.clientX;
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    apply(lastX);
  });
}`,
      },
    ],
    edgeCases: [
      "Every callback in a frame gets the same timestamp, the `performance.now()` value at the start of that frame's pump.",
      "A callback registered from inside a callback runs in the next frame.",
      "`cancelAnimationFrame` called from a callback does not stop another callback of the same frame; that one still runs. It does stop callbacks registered for the next frame.",
      "While the window is minimized, hidden or fully covered, no callbacks run; they wait for the next drawn frame. Timers keep running.",
    ],
  },
  "request-idle-callback": {
    examples: [
      {
        title: "Build a search index without blocking typing",
        lang: "ts",
        code: `export function indexInBackground(docs: { id: string; text: string }[], index: Map<string, string[]>) {
  let next = 0;
  const work = (deadline: IdleDeadline) => {
    while (next < docs.length && deadline.timeRemaining() > 0) {
      const doc = docs[next++]!;
      for (const word of doc.text.toLowerCase().split(/\\W+/)) {
        if (word) index.set(word, [...(index.get(word) ?? []), doc.id]);
      }
    }
    if (next < docs.length) requestIdleCallback(work);
  };
  requestIdleCallback(work);
}`,
      },
    ],
    edgeCases: [
      "The callbacks run after each frame's `requestAnimationFrame` callbacks, every drawn frame.",
      "The budget is about 4 ms, shared by every idle callback of that frame; `timeRemaining()` counts down from it and stops at `0`.",
      "The `timeout` option is ignored and `didTimeout` is always `false`.",
      "While the window draws no frames (minimized or hidden), idle callbacks wait for the next drawn frame; a browser still runs them, late.",
    ],
  },
  "structured-clone": {
    examples: [
      {
        title: "Snapshot a Vue form for undo",
        lang: "ts",
        code: `import { reactive, toRaw } from "vue";

const form = reactive({ title: "", tags: [] as string[], due: new Date() });
const history: (typeof form)[] = [];

export function checkpoint() {
  history.push(structuredClone(toRaw(form)));
}

export function undo() {
  const previous = history.pop();
  if (previous) Object.assign(form, previous);
}`,
        note: "A reactive object is a `Proxy`, which cannot be cloned; `toRaw` hands over the plain object.",
      },
      {
        title: "Hand a large buffer to a consumer without copying it twice",
        lang: "ts",
        code: `export function takeFrame(pixels: ArrayBuffer) {
  const owned = structuredClone(pixels, { transfer: [pixels] });
  console.log(pixels.byteLength, owned.byteLength);
  return owned;
}`,
        note: "After the transfer the original buffer is detached and its `byteLength` is `0`.",
      },
    ],
    edgeCases: [
      "A `Proxy`, including any Vue `reactive()` object, throws a `DataCloneError` (`A Proxy could not be cloned.`); so do functions, promises and symbols.",
      "Class instances come back as plain objects; `Date`, `Map`, `Set`, `RegExp`, typed arrays, `Blob`, `File` and `Error` objects keep their type.",
      "Cycles and shared references are preserved: an object reachable twice is cloned once.",
      "A graph too deep to walk throws a `RangeError`, as in a browser, instead of crashing the process.",
      "Transferring an `ArrayBuffer` that is already detached throws a `DataCloneError`; any failure leaves every listed buffer intact.",
    ],
  },
  "report-error": {
    examples: [
      {
        title: "Report a failed autosave without stopping the loop",
        lang: "ts",
        code: `export async function autosaveAll(drafts: { id: string; save(): Promise<void> }[]) {
  for (const draft of drafts) {
    try {
      await draft.save();
    } catch (e) {
      reportError(e);
    }
  }
}`,
        note: "Each failure reaches your `error` listeners and the host's error log, and the next draft is still saved.",
      },
    ],
    edgeCases: [
      "The `error` event is dispatched synchronously, before `reportError` returns.",
      "The event's `message` is prefixed with `Uncaught `, for example `Uncaught TypeError: boom`.",
      "Calling `preventDefault()` on the event, or returning `true` from `onerror`, keeps the error out of the host's log.",
      "Calling it with no argument throws a `TypeError`.",
    ],
  },
  "error-event": {
    examples: [
      {
        title: "Send reported errors to your crash reporter",
        lang: "ts",
        code: `addEventListener("error", (e: ErrorEvent) => {
  void fetch("https://crash.example.com/report", {
    method: "POST",
    body: JSON.stringify({
      message: e.message,
      file: e.filename,
      line: e.lineno,
      stack: e.error instanceof Error ? e.error.stack : null,
    }),
  });
});`,
      },
    ],
    edgeCases: [
      "A constructed `ErrorEvent` has `error` set to `null` unless the init dictionary gives one.",
      "`lineno` and `colno` are truncated to whole numbers: `colno: 4.9` reads `4`.",
      "Reading `message` from an object that is not an `ErrorEvent` throws a `TypeError`.",
      "A `ResizeObserver` loop error arrives as an `ErrorEvent` with the message `ResizeObserver loop completed with undelivered notifications.`",
    ],
  },
  onerror: {
    examples: [
      {
        title: "Show a banner for errors your code reports",
        lang: "ts",
        code: `import { ref } from "vue";

export const lastError = ref<string | null>(null);

window.onerror = (message, source, line, column, error) => {
  lastError.value = error instanceof Error ? error.message : String(message);
  return true;
};`,
        note: "Returning `true` marks the error as handled, so the host does not log it again.",
      },
    ],
    edgeCases: [
      "It is called with the five HTML arguments `(message, source, lineno, colno, error)`, not with the event.",
      "An exception thrown inside `onerror` itself is logged as a listener error.",
    ],
  },
  "performance-now": {
    examples: [
      {
        title: "Measure how long a sync takes",
        lang: "ts",
        code: `export async function timed<T>(label: string, work: () => Promise<T>): Promise<T> {
  const start = performance.now();
  try {
    return await work();
  } finally {
    console.log(label + ": " + (performance.now() - start).toFixed(1) + " ms");
  }
}`,
      },
    ],
    edgeCases: [
      "It shares one monotonic clock with `requestAnimationFrame` timestamps and timer deadlines, so a rAF timestamp can be compared with it directly.",
    ],
  },
  "performance-time-origin": {
    examples: [
      {
        title: "Turn a high-resolution timestamp into a wall-clock time",
        lang: "ts",
        code: `export function wallClock(highRes: number): string {
  return new Date(performance.timeOrigin + highRes).toISOString();
}

requestAnimationFrame((t) => console.log("first frame at", wallClock(t)));`,
      },
    ],
    edgeCases: ["It is a Unix time in milliseconds, fixed when the JS runtime started."],
  },
  "performance-mark": {
    edgeCases: ['`performance.mark` is not defined; check `typeof performance.mark === "function"` before calling it.'],
  },
  "performance-measure": {
    edgeCases: ["`performance.measure` is not defined; measure a span with two `performance.now()` readings instead."],
  },
  "broadcast-channel": {
    examples: [
      {
        title: "Tell every open view that the user signed out",
        lang: "ts",
        code: `import { useRouter } from "vue-router";

const session = new BroadcastChannel("session");

export function signOut() {
  localStorage.removeItem("token");
  session.postMessage({ type: "signed-out" });
}

export function useSignOutRedirect() {
  const router = useRouter();
  const view = new BroadcastChannel("session");
  view.onmessage = (e: MessageEvent<{ type: string }>) => {
    if (e.data.type === "signed-out") void router.push("/sign-in");
  };
  return view;
}`,
        note: "The sender does not receive its own message; every other channel object with the same name does.",
      },
      {
        title: "Close a channel when its component unmounts",
        lang: "vue",
        code: `<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";

const theme = ref("light");
const channel = new BroadcastChannel("theme");
channel.addEventListener("message", (e) => (theme.value = e.data));

onBeforeUnmount(() => channel.close());
</script>

<template>
  <div class="preview" :class="theme">Preview</div>
</template>`,
      },
    ],
    edgeCases: [
      "The value is cloned once when posted, and each receiver gets its own copy on the next pump, never synchronously.",
      "A value that cannot be cloned throws a `DataCloneError` from `postMessage`; a closed channel throws an `InvalidStateError`.",
      "A channel closed before the message is delivered receives nothing.",
      "Components in an `<SkWindow>` share the app's one runtime, so channels there reach the main window. A second process of the app does not receive them.",
    ],
  },
};
