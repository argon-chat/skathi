export type HapticPattern =
  | "generic"
  | "alignment"
  | "level-change"
  | "hover"
  | "click"
  | "press"
  | "release"
  | "success"
  | "error"
  | "collide"
  | "grow"
  | "ink-continuous"
  | "pencil-continuous"
  | "brush-continuous"
  | "eraser-continuous";

export interface HapticOptions {
  pointerId?: number;
  when?: "now" | "next-frame";
  /** Play only the requested pattern, or try this one alternative if it is unavailable. */
  fallback?: HapticPattern | false;
}

export type HapticTrigger = "press" | "activate" | "change" | "step" | "boundary" | "snap" | "hover";

export interface AppHaptics {
  readonly supported: boolean;
  supportsPointer(pointerId: number): boolean;
  /** Currently advertised patterns, without playing a pulse. A pen can advertise textures before contact. */
  getSupportedPatterns(pointerId?: number): HapticPattern[];
  /** Resolve the element's CSS binding when an app-defined control reaches a gesture such as a guide snap. */
  trigger(element: Element, gesture: HapticTrigger, pointerId?: number): boolean;
  /** True if the window accepted the effect for immediate or frame-synchronized delivery. */
  perform(pattern: HapticPattern, options?: HapticOptions): boolean;
  /** Stop continuous feedback or cancel queued Windows feedback for this pointer. */
  stop(pointerId: number): boolean;
}

declare const __sk_appHaptics: AppHaptics;
export const appHaptics: AppHaptics = __sk_appHaptics;
