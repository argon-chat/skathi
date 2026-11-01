// A fixture for packages/vn-test/specs/mic-consent.spec.ts (m26 voice-fixes-1
// item 1, docs/PERMISSIONS.md §5.1): getUserMedia behind the app's own
// permission prompt.
//
// The spec launches it with VN_MEDIA_CAPTURE=fake, so the microphone is a
// synthetic 440 Hz tone and no device of the machine's is ever opened, and
// with VN_DIALOGS=scripted (vn-test's default), so the prompt is a question
// the spec answers through the devtools and never reaches the screen.
//
// `__VN_TEST_MIC__` holds what the spec drives from `eval`:
// - `appPermissions`, the vue-native extension (tier 2);
// - `changes`, one `<state>` per `change` on the microphone's PermissionStatus;
// - `open(constraints?)`, a getUserMedia whose result lands in `results` as
//   `ok:<readyState>:<label>` or `err:<name>: <message>`, the track kept in
//   `tracks` until `stopAll()`, and its processing in `settings`.
import { appPermissions, createApp, h } from "vue-native";

const results: string[] = [];
/** Per successful open: `getSettings()`'s `echoCancellation|noiseSuppression|autoGainControl|channelCount`. */
const settings: string[] = [];
const changes: string[] = [];
const tracks: MediaStreamTrack[] = [];

navigator.permissions.query({ name: "microphone" as PermissionName }).then((st) => {
  st.addEventListener("change", () => changes.push(st.state));
});

function open(constraints: MediaStreamConstraints = { audio: true }) {
  navigator.mediaDevices.getUserMedia(constraints).then(
    (ms) => {
      const t = ms.getAudioTracks()[0];
      tracks.push(t);
      const s = t.getSettings();
      settings.push(`${s.echoCancellation}|${s.noiseSuppression}|${s.autoGainControl}|${s.channelCount}`);
      results.push(`ok:${t.readyState}:${t.label}`);
    },
    (e: DOMException) => results.push(`err:${e.name}: ${e.message}`),
  );
}

function stopAll() {
  for (const t of tracks.splice(0)) t.stop();
}

(globalThis as unknown as { __VN_TEST_MIC__: unknown }).__VN_TEST_MIC__ = {
  appPermissions,
  changes,
  results,
  settings,
  tracks,
  open,
  stopAll,
};

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "12px", background: "#ffffff" } }, [
        h("button", { id: "open", onClick: () => open() }, "getUserMedia"),
      ]);
  },
}).mount();
