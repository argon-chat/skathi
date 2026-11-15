/** A small straight-RGBA icon. Pixels are copied synchronously by set(). */
export interface TrayPixels {
  readonly width: number;
  readonly height: number;
  readonly data: Uint8Array | Uint8ClampedArray;
  /** Draw the alpha mask in the taskbar foreground colour (a menu-bar template image on macOS). */
  readonly template?: boolean;
}
export type TrayMenuItem =
  | { readonly type: "separator" }
  | {
      readonly label: string;
      readonly disabled?: boolean;
      readonly checked?: boolean;
      readonly default?: boolean;
      readonly onSelect?: () => void;
    }
  | { readonly label: string; readonly disabled?: boolean; readonly submenu: readonly TrayMenuEntry[] }
  | { readonly role: "quit"; readonly label?: string; readonly disabled?: boolean };
export type TrayMenuEntry = TrayMenuItem | false | null | undefined;
export interface TrayOptions {
  /** Packaged asset or data URL, or RGBA pixels. SVG assets/data URLs are template icons. */
  readonly icon?: string | TrayPixels;
  readonly tooltip?: string;
  readonly menu?: readonly TrayMenuEntry[];
  /** Without a callback a primary click opens the menu. */
  readonly onClick?: () => void;
}
export interface AppTray {
  /** Windows and macOS. Unsupported hosts throw NotSupportedError on set(options). */
  readonly available: boolean;
  /** Replace the complete tray model; null removes it. Native updates coalesce. */
  set(options: TrayOptions | null): void;
}
declare const __sk_appTray: AppTray;
export const appTray: AppTray = __sk_appTray;
