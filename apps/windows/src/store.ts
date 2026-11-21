import { reactive } from "vue";
import { appWindow } from "skathi:app";

/** Which page is shown. Bumping `key` remounts it with fresh state. */
export const nav = reactive({ id: "windows/state-verbs", key: 0 });

export function go(id: string, fresh = false): void {
  if (nav.id === id && !fresh) return;
  nav.id = id;
  nav.key++;
}

/** The title bar: the app's own caption buttons, or document tabs next to the host cluster. */
export const titleBar = reactive({ tabs: false, tab: "Notes" });

/** The always-on-top page shrinks the window into a notes window. */
export const compact = reactive({
  on: false,
  saved: null as SkWindowRect | null,
  reference: false,
  note: "Ship the window guides\n- record the clips on the second monitor\n- check the posters\n- link the API pages",
});

export function enterCompact(): void {
  if (compact.on) return;
  const r = appWindow.getContentBounds();
  compact.saved = r;
  compact.on = true;
  appWindow.setContentBounds({ x: r.x + r.width - 420, y: r.y + 120, width: 360, height: 440 });
}

export function leaveCompact(): void {
  compact.reference = false;
  appWindow.alwaysOnTop = false;
  if (!compact.on) return;
  compact.on = false;
  if (compact.saved) appWindow.setContentBounds(compact.saved);
  compact.saved = null;
}

const DRAFT_KEY = "windows.draft";
const SAVED_DRAFT = `Window guides, review notes

The state verbs read well. Add the macOS toggle note next to the table.
Pop-out tooltips: say that mapFromMain() is Windows only before the sample, not after.`;

/** The note on the unsaved-changes page; the close flow in closing.ts asks while it differs from `saved`. */
export const draft = reactive({
  text: readDraft(),
  saved: readDraft(),
  asking: false,
});

function readDraft(): string {
  try {
    return localStorage.getItem(DRAFT_KEY) ?? SAVED_DRAFT;
  } catch {
    return SAVED_DRAFT;
  }
}

export function saveDraft(): void {
  try {
    localStorage.setItem(DRAFT_KEY, draft.text);
  } catch {}
  draft.saved = draft.text;
}

export function resetDraft(): void {
  draft.text = draft.saved = SAVED_DRAFT;
  draft.asking = false;
  try {
    localStorage.removeItem(DRAFT_KEY);
  } catch {}
}

/** Hide-to-background: the close button hides the window instead of quitting. */
export const background = reactive({ keepRunning: false, returns: 0, lastReturn: "" });
