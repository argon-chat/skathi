import { appPermissions, createApp, h } from "@skathi-native/runtime";

const results: string[] = [];
const listed: string[] = [];
const settings: string[] = [];
const changes: string[] = [];
const tracks: MediaStreamTrack[] = [];

navigator.permissions.query({ name: "microphone" }).then((st) => {
  st.addEventListener("change", () => changes.push(st.state));
});

function open(constraints: MediaStreamConstraints = { audio: true }) {
  navigator.mediaDevices.getUserMedia(constraints).then(
    (ms) => {
      const t = ms.getAudioTracks()[0]!;
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

async function localDevices(kind: string, request: boolean): Promise<MediaDeviceInfo[]> {
  if (request && kind === "audioinput") {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    for (const track of stream.getTracks()) track.stop();
  }
  return (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === kind);
}

function devices(kind: string, request = true) {
  localDevices(kind, request).then(
    (ds) => listed.push("ok:" + ds.map((d) => `${d.deviceId}/${d.label}`).join(",")),
    (e: DOMException) => listed.push(`err:${e.name}: ${e.message}`),
  );
}

(globalThis as unknown as { __SK_TEST_MIC__: unknown }).__SK_TEST_MIC__ = {
  appPermissions,
  changes,
  devices,
  listed,
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
