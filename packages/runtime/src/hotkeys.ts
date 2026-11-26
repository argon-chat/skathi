/** Device 0 is the keyboard, device 1 the mouse (1 left, 2 right, 3 middle, 4 and 5 the side buttons). */
export interface HotkeyButton {
  readonly device: number;
  /** A Windows virtual-key code on every platform, or 0x10000 | scan code for keys a VK cannot tell apart. */
  readonly code: number;
}
export interface HotkeyChord {
  readonly buttons: readonly HotkeyButton[];
}
export interface HotkeyBinding {
  /** Unique, at most 128 bytes. */
  readonly id: string;
  readonly chord: HotkeyChord;
  /** Consume the keys that complete the chord, so the focused app does not see them. */
  readonly suppress?: boolean;
  /** Minimum time between two down edges, at most 60000. */
  readonly triggerCooldownMs?: number;
}
export interface HotkeyStatus {
  readonly running: boolean;
  readonly platform: string;
  /** macOS Accessibility access; always true on Windows. */
  readonly accessibilityGranted: boolean;
  readonly error: string | null;
  readonly restarts: number;
}
/** Every delivered "down" gets its "up", also when the bindings change, a capture starts or the hooks restart. */
export interface HotkeyEvent {
  readonly id: string;
  readonly phase: "down" | "up";
  /** True on a release synthesized when the service cancels an active chord. */
  readonly cancelled?: true;
}
export interface AppHotkeys {
  /** Windows and macOS. */
  readonly available: boolean;
  /** Replaces every binding; false when nothing changed. The first call starts the system hooks. */
  setBindings(bindings: readonly HotkeyBinding[]): boolean;
  status(): HotkeyStatus;
  /** Shows the macOS Accessibility prompt; call it from a user action. */
  requestAccessibility(): boolean;
  restart(): boolean;
  /**
   * Records the next chord, finished when its keys are released. Rejects with
   * "hotkey capture cancelled" (Esc or cancelCapture), "hotkey capture timeout" or
   * "hotkey capture superseded".
   */
  captureOnce(): Promise<HotkeyChord>;
  cancelCapture(): void;
  onFired(listener: (event: HotkeyEvent) => void): () => void;
}
declare const __sk_appHotkeys: AppHotkeys;
export const appHotkeys: AppHotkeys = __sk_appHotkeys;
