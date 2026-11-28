import type { WebEntry } from "./entries";
import { browserCompatData, findCompat, type BcdData } from "../browser-compat";

export const overrides: Readonly<Record<string, string | null>> = {
  "request-animation-frame": "api.Window.requestAnimationFrame",
  "request-idle-callback": "api.Window.requestIdleCallback",
  onerror: "api.Window.error_event",
  "performance-now": "api.Performance.now",
  "performance-time-origin": "api.Performance.timeOrigin",
  "performance-mark": "api.Performance.mark",
  "performance-measure": "api.Performance.measure",
  "web-workers": "api.Worker",
  "audio-worklet": "api.AudioWorklet",
  "elements-and-attributes": "api.Element",
  "get-computed-style": "api.Window.getComputedStyle",
  "element-from-point": "api.Document.elementFromPoint",
  "scroll-into-view": "api.Element.scrollIntoView",
  "match-media": "api.Window.matchMedia",
  contenteditable: "html.global_attributes.contenteditable",
  "fullscreen-api": "api.Element.requestFullscreen",
  "web-animations": "api.Element.animate",
  "view-transitions": "api.Document.startViewTransition",
  "smooth-scrolling": "api.Element.scrollTo.options_behavior_parameter",
  "pointer-lock": "api.Element.requestPointerLock",
  "pointer-event-pointer-type": "api.PointerEvent.pointerType",
  "pointer-event-pressure": "api.PointerEvent.pressure",
  "pointer-event-tangential-pressure": "api.PointerEvent.tangentialPressure",
  "pointer-event-tilt-x": "api.PointerEvent.tiltX",
  "pointer-event-altitude-angle": "api.PointerEvent.altitudeAngle",
  "pointer-event-twist": "api.PointerEvent.twist",
  "pointer-event-get-coalesced-events": "api.PointerEvent.getCoalescedEvents",
  "pointer-event-get-predicted-events": "api.PointerEvent.getPredictedEvents",
  pointerrawupdate: "api.Element.pointerrawupdate_event",
  "document-fonts": "api.Document.fonts",
  "custom-elements": "api.CustomElementRegistry",
  "shadow-dom": "api.ShadowRoot",
  alert: "api.Window.alert",
  confirm: "api.Window.confirm",
  prompt: "api.Window.prompt",
  "window-open": "api.Window.open",
  "accept-language": "http.headers.Accept-Language",
  "navigator-send-beacon": "api.Navigator.sendBeacon",
  "navigator-connection": "api.Navigator.connection",
  "service-workers": "api.ServiceWorker",
  "cache-api": "api.Cache",
  "crypto-get-random-values": "api.Crypto.getRandomValues",
  "crypto-random-uuid": "api.Crypto.randomUUID",
  "local-storage": "api.Window.localStorage",
  "session-storage": "api.Window.sessionStorage",
  "indexed-db": "api.indexedDB",
  "navigator-storage": "api.Navigator.storage",
  "file-system-access": "api.Window.showOpenFilePicker",
  "input-type-file": "html.elements.input.type_file",
  "input-type-color": "html.elements.input.type_color",
  "html-input-element-show-picker": "api.HTMLInputElement.showPicker",
  showpicker: null,
  "a-download": "html.elements.a.download",
  "navigator-locks": "api.Navigator.locks",
  "canvas-2d": "api.CanvasRenderingContext2D",
  webgpu: "api.GPU",
  webgl: "api.WebGLRenderingContext",
  "webgl-2": "api.WebGL2RenderingContext",
  "web-audio": "api.AudioContext",
  "get-user-media": "api.MediaDevices.getUserMedia",
  video: "html.elements.video",
  "navigator-media-session": "api.Navigator.mediaSession",
  webcodecs: null,
  webrtc: "api.RTCPeerConnection",
  "navigator-language": "api.Navigator.language",
  "navigator-languages": "api.Navigator.languages",
  "navigator-user-agent-data": "api.Navigator.userAgentData",
  "navigator-permissions-query": "api.Permissions.query",
  "async-clipboard": "api.Clipboard",
  "navigator-get-battery": "api.Navigator.getBattery",
  "navigator-user-activation": "api.Navigator.userActivation",
  "navigator-window-controls-overlay": "api.Navigator.windowControlsOverlay",
  "launch-queue": "api.Window.launchQueue",
  "navigator-set-app-badge": "api.Navigator.setAppBadge",
  "navigator-clear-app-badge": "api.Navigator.clearAppBadge",
  "navigator-wake-lock": "api.Navigator.wakeLock",
  "navigator-share": "api.Navigator.share",
  "navigator-can-share": "api.Navigator.canShare",
  passkeys: "api.PublicKeyCredential",
  gamepad: "api.Navigator.getGamepads",
  geolocation: "api.Geolocation",
  vibration: "api.Navigator.vibrate",
  "web-bluetooth": "api.Bluetooth",
  webusb: "api.USB",
  webhid: "api.HID",
  "web-serial": "api.Serial",
};

function candidates(e: WebEntry): string[] {
  if (e.slug in overrides) {
    const k = overrides[e.slug];
    return k === null || k === undefined ? [] : [k];
  }
  if (e.skathiOnly) return [];
  switch (e.kind) {
    case "interface":
    case "function":
      return [`api.${e.name}`];
    default:
      return [];
  }
}

export function compat(entry: WebEntry, data: BcdData = browserCompatData) {
  return findCompat(data, candidates(entry), 1);
}
