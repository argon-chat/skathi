import { onMounted, onUnmounted, ref, shallowRef } from "vue";
import { describe } from "./engine";

// Both readouts display hundredths. Keep their refresh independent of the monitor's frame rate.
export const gamepadReadoutInterval = 34;

export interface ShowcaseGamepadEvent {
  type: "gamepadconnected" | "gamepaddisconnected";
  index: number;
  id: string;
  timestamp: number;
}

function sameReading(left?: Gamepad | null, right?: Gamepad | null) {
  return left?.id === right?.id && left?.timestamp === right?.timestamp && left?.connected === right?.connected;
}

export function useShowcaseGamepads() {
  const available = typeof navigator.getGamepads === "function";
  const slots = shallowRef<readonly (Gamepad | null)[]>([]);
  const events = shallowRef<readonly ShowcaseGamepadEvent[]>([]);
  const hapticStatus = ref("");
  let timer: ReturnType<typeof setTimeout> | undefined;
  let focused = true;
  let effectSlot: number | undefined;
  let command = 0;

  function clearHapticStatus() {
    ++command;
    effectSlot = undefined;
    hapticStatus.value = "";
  }

  function queueSample(delay = gamepadReadoutInterval) {
    if (timer === undefined && focused && !document.hidden) timer = setTimeout(sample, delay);
  }

  function sample() {
    timer = undefined;
    const next = navigator.getGamepads?.() ?? [];
    const previous = slots.value;
    // Polls return fresh objects even when all controls are idle.
    if (next.length !== previous.length || next.some((pad, i) => !sameReading(pad, previous[i]))) {
      slots.value = next.map((pad, i) => (sameReading(pad, previous[i]) ? (previous[i] ?? null) : pad));
    }
    if (next.some((pad) => pad?.connected)) queueSample();
  }

  function pause() {
    clearTimeout(timer);
    timer = undefined;
    if (slots.value.length) slots.value = [];
  }

  function onFocus() {
    focused = true;
    queueSample(0);
  }

  function onBlur() {
    focused = false;
    pause();
  }

  function onVisibilityChange() {
    if (document.hidden) pause();
    else queueSample(0);
  }

  function onConnection(event: GamepadEvent) {
    const pad = event.gamepad;
    if (!pad || (event.type !== "gamepadconnected" && event.type !== "gamepaddisconnected")) return;
    if (event.type === "gamepaddisconnected" && effectSlot === pad.index) clearHapticStatus();
    events.value = [
      { type: event.type, index: pad.index, id: pad.id, timestamp: event.timeStamp },
      ...events.value.slice(0, 11),
    ];
    queueSample(0);
  }

  onMounted(() => {
    if (!available) return;
    window.addEventListener("gamepadconnected", onConnection);
    window.addEventListener("gamepaddisconnected", onConnection);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", onVisibilityChange);
    queueSample(0);
  });
  onUnmounted(() => {
    ++command;
    clearTimeout(timer);
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("blur", onBlur);
    document.removeEventListener("visibilitychange", onVisibilityChange);
    window.removeEventListener("gamepadconnected", onConnection);
    window.removeEventListener("gamepaddisconnected", onConnection);
  });

  async function rumble(index: number, stop: boolean) {
    const current = ++command;
    effectSlot = index;
    const pad = slots.value[index];
    if (!pad?.connected) {
      hapticStatus.value = "Controller disconnected";
      return;
    }
    const actuator = pad.vibrationActuator;
    if (!actuator?.effects.includes("dual-rumble")) {
      hapticStatus.value = "Dual rumble is unavailable";
      return;
    }
    hapticStatus.value = stop ? "Stopping vibration" : "Playing vibration";
    try {
      const result = stop
        ? await actuator.reset()
        : await actuator.playEffect("dual-rumble", { duration: 250, strongMagnitude: 0.45, weakMagnitude: 0.2 });
      if (current === command) hapticStatus.value = result;
    } catch (error) {
      if (current === command) hapticStatus.value = describe(error);
    }
  }

  return {
    available,
    slots,
    events,
    hapticStatus,
    clearHapticStatus,
    playRumble: (index: number) => rumble(index, false),
    resetRumble: (index: number) => rumble(index, true),
  };
}
