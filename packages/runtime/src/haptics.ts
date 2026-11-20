export type HapticPattern = "generic" | "alignment" | "level-change" | "ink-continuous" | "pencil-continuous";

export interface AppHaptics {
  readonly supported: boolean;
  supportsPointer(pointerId: number): boolean;
  /** True if the window accepted the effect for immediate or frame-synchronized delivery. */
  perform(pattern: HapticPattern, options?: { pointerId?: number; when?: "now" | "next-frame" }): boolean;
  /** Stop continuous feedback or cancel queued Windows feedback for this pointer. */
  stop(pointerId: number): boolean;
}

declare const __sk_appHaptics: AppHaptics;
export const appHaptics: AppHaptics = __sk_appHaptics;
