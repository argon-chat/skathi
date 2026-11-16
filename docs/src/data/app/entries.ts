import { capture } from "./capture";
import { devtools } from "./devtools";
import { host } from "./host";
import { hotkeys } from "./hotkeys";
import { launch } from "./launch";
import { locale } from "./locale";
import { permissions } from "./permissions";
import { categories, type AppEntry, type Category } from "./shared";
import { shell } from "./shell";
import { storage } from "./storage";
import { tray } from "./tray";

export { categories };
export type { AppEntry, Category };

export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {
  Tray: "`appTray` puts one icon in the Windows notification area or the macOS menu bar, with a tooltip, a menu and a click handler; each `set()` replaces the whole model. Linux has no tray: `available` is `false`. See [Closing and running in the background](/guides/closing-and-background/).",
  "Global hotkeys":
    "`appHotkeys` watches the keyboard and mouse system-wide, also while another app has the focus, and reports a `down` and an `up` for each binding. Keys are Windows virtual-key codes on every platform. On macOS the app needs Accessibility access; Linux has no global hotkeys.",
  Storage:
    "`appStorage` says where `localStorage` and IndexedDB keep their data, moves them, and encrypts them at rest with the platform's key store. It also names the user's standard folders.",
  Shell:
    "`appShell` hands URLs and files to the system: open with the default app, reveal in the file manager, move to the trash. Every call returns a promise and rejects with an `Error` that has a Node-style `code`. Implemented on Windows and Linux; on macOS the calls reject for now.",
  Permissions:
    "`appPermissions` is `navigator.permissions` with a standalone `request()` and a fourth state, `unsupported`, for a permission the platform has no switch for. On Windows, Skathi asks for the microphone with its own dialog, since the system never asks a desktop app.",
  "Screen capture":
    "`appCapture` lists screens and windows with thumbnails for a screen-share picker, and reads and requests the microphone, camera and screen capture permissions under capture names. Windows and macOS.",
  Locale:
    "`appLocale` chooses the language of the SDK's own dialogs and of the app's strings, and looks up text with `{name}` placeholders. It is not reactive: keep the language in your own reactive state.",
  Launch:
    "`appLaunch` is what a browser's `launchQueue` cannot say: the raw command line, the working folder, the single-instance lock, deep links and custom URL schemes, and a `launch` event when the app is started again. Arguments and URLs are untrusted input.",
  Embedding:
    "`appHost` is the app's side of a program that runs Skathi as a library (`sk_app::embed`): call its functions, receive its events. In a standalone app it is inert, with `available` set to `false`.",
  Devtools:
    "`appDevtools` turns the devtools listener off and on at run time and opens the inspector from code. It exists in every build; without devtools it answers `false`. See [Devtools](/guides/devtools/).",
};

export const entries: readonly AppEntry[] = [
  ...tray,
  ...hotkeys,
  ...storage,
  ...shell,
  ...permissions,
  ...capture,
  ...locale,
  ...launch,
  ...host,
  ...devtools,
];
