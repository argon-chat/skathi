export const WINDOWS = process.platform === "win32";
export const MACOS = process.platform === "darwin";
export const LINUX = process.platform === "linux";

/** gpui's Linux backend has no `render_to_image`, so a Linux host takes no screenshot. */
export const SCREENSHOTS = !LINUX;

/** What is on screen: PrintWindow on Windows, elsewhere the last scene rendered again. */
export const ON_SCREEN = WINDOWS ? "print_window" : "render_to_image";
