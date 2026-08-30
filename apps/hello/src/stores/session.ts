// A real pinia store — the setup-function flavour, byte-for-byte what you
// would write in a browser app. Nothing in here knows it is running on gpui.
//
// It exists to prove two independent things about state:
//   * <KeepAlive> preserves a page's LOCAL state (refs inside the page's own
//     setup) by keeping the instance alive across route changes;
//   * pinia preserves SHARED state regardless — the store lives in the app's
//     effect scope, so it outlives every page, KeepAlive'd or not (the Lab tab
//     is deliberately excluded from KeepAlive and still reads the same value).
import { computed, ref } from "vue";
import { defineStore } from "pinia";

export const useSession = defineStore("session", () => {
  const user = ref("guest");
  const visits = ref(0);
  // Explicit element type: `ref([])` infers `never[]`, so every push into it
  // would be an error rather than the string list this actually holds.
  const notes = ref<string[]>([]);

  // A computed inside the store: recomputes for every subscriber at once.
  const badge = computed(() => `${user.value} · ${visits.value}`);
  const noteCount = computed(() => notes.value.length);

  function bump() {
    visits.value += 1;
  }
  function reset() {
    visits.value = 0;
    notes.value = [];
  }
  // `unknown`, not `string`: both of these already coerce with `String(...)`,
  // which is the browser-Vue idiom for "whatever a template handed me". Saying
  // `string` would delete the coercion's reason for existing.
  function addNote(text: unknown) {
    const t = String(text).trim();
    if (!t) return;
    notes.value = [...notes.value.slice(-2), t];
  }
  function setUser(name: unknown) {
    user.value = String(name || "guest");
  }

  return { user, visits, notes, badge, noteCount, bump, reset, addNote, setUser };
});
