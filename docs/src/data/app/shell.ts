import { OK, PLAN, all, importLine, os, section } from "./shared";

const IMPORT = importLine("appShell");
const PLATFORMS = os(OK, PLAN, OK);
const MAC =
  'On macOS, every call rejects with an `Error` named `NotSupportedError` (`code: "ENOSYS"`): not implemented yet.';
const LINUX_TOOLS =
  "On Linux, the calls run the freedesktop tools and wait for them: `xdg-open`, the file manager's D-Bus interface and `gio trash`. A missing tool rejects with `NotSupportedError` (`ENOSYS`), one that fails with `OperationError`.";
const PATH_RULES =
  "The path must be absolute; a relative path, an empty one, a NUL or a Windows alternate data stream (`file.txt:stream`) rejects with a `TypeError`. A path that does not exist rejects with `NotFoundError` (`ENOENT`).";

export const shell = section("Shell", [
  {
    slug: "app-shell",
    name: "appShell",
    kind: "property",
    summary:
      "Opens URLs and files with their default app, reveals a file in the file manager, and moves files to the trash.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}

appShell.openExternal(url: string): Promise<void>
appShell.openPath(path: string, options?: { allowExecutable?: boolean }): Promise<void>
appShell.showItemInFolder(path: string): Promise<void>
appShell.trashItem(path: string): Promise<void>`,
    notes: [
      "Like Electron's `shell`, with two safer defaults: `openPath()` refuses programs and scripts unless asked, and `openExternal()` refuses `file:` URLs, so a URL is never a way around that check.",
      "Every call returns a promise and never throws: a bad argument rejects with a `TypeError`, an OS failure with an [AppDesktopError](/app-api/app-desktop-error/).",
      "The OS work runs on a thread of its own, one call after another, so the UI never waits on the shell or the disk.",
      "With the environment variable `SK_SHELL=record`, calls are checked as usual and then resolve without reaching the OS, each logging a `[shell]` line. Use it in tests.",
      "Links in content (`<a href>`, `window.open`) open only `http`, `https` and `mailto`; `appShell.openExternal()` is the way to open another scheme the app trusts.",
    ],
    examples: [
      {
        title: "Open the log folder from a Help menu",
        lang: "ts",
        code: `import { appShell, appStorage } from "@skathi-native/runtime";

export async function openLogs(): Promise<void> {
  try {
    await appShell.openPath(appStorage.getUserFolder("LOCALAPPDATA", "Notes/logs"));
  } catch (error) {
    console.warn("Could not open the logs:", error);
  }
}`,
      },
    ],
    limitations: [MAC, LINUX_TOOLS],
    related: [
      "shell-open-external",
      "shell-open-path",
      "shell-show-item-in-folder",
      "shell-trash-item",
      "app-desktop-error",
    ],
    aliases: ["shell", "Electron shell", "opener"],
  },
  {
    slug: "shell-open-external",
    name: "appShell.openExternal()",
    kind: "method",
    summary:
      "Opens a URL with the app the system registers for its scheme: the browser for `https:`, the mail app for `mailto:`, and so on.",
    platforms: PLATFORMS,
    syntax: "appShell.openExternal(url: string): Promise<void>",
    params: [{ name: "url", type: "string", description: "An absolute URL with any scheme but `file:`." }],
    returns:
      "A promise that resolves once the shell has accepted the URL, not when the other app has finished opening it.",
    notes: [
      "Custom schemes (`zoommtg:`, `ms-settings:privacy`, another app's protocol) are allowed. Treat a URL built from user or network input with care: it starts whatever handles its scheme.",
    ],
    examples: [
      {
        title: "Open a link in the default browser",
        lang: "vue",
        code: `<script setup lang="ts">
import { appShell } from "@skathi-native/runtime";

const props = defineProps<{ href: string }>();
</script>

<template>
  <a :href="props.href" @click.prevent="appShell.openExternal(props.href)"><slot /></a>
</template>`,
      },
    ],
    edgeCases: [
      "A `file:` URL rejects with a `TypeError` that points to `openPath()`; so does a Windows path such as `C:\\x`, which looks like a one-letter scheme.",
      "An empty URL, one that is not absolute, or one with a control character rejects with a `TypeError`.",
      "On Windows, a scheme no app handles rejects with `NotFoundError` (`ENOASSOC`).",
    ],
    limitations: [MAC, LINUX_TOOLS],
    related: ["shell-open-path", "app-shell", "launch-register-protocol"],
    aliases: ["shell.openExternal", "open URL", "open in browser"],
  },
  {
    slug: "shell-open-path",
    name: "appShell.openPath()",
    kind: "method",
    summary:
      "Opens a file with its default app, or a folder in the file manager. Programs and scripts are refused unless allowed.",
    platforms: PLATFORMS,
    syntax: "appShell.openPath(path: string, options?: { allowExecutable?: boolean }): Promise<void>",
    params: [
      { name: "path", type: "string", description: "An absolute path to a file or folder." },
      {
        name: "options.allowExecutable",
        type: "boolean",
        description: "Allow opening a program, installer, shortcut or script, which runs it.",
        optional: true,
        default: "false",
      },
    ],
    returns: "A promise that resolves once the shell has accepted the file.",
    notes: [
      "Without `allowExecutable`, a file Windows would run rather than open is refused on every platform: `.exe`, `.msi`, `.bat`, `.cmd`, `.ps1`, `.js`, `.vbs`, `.lnk`, `.url`, `.reg`, `.jar` and about sixty more, plus every extension in `PATHEXT`. The extension is read as Windows reads it, so `report.pdf.exe.` counts as `.exe`.",
      "On Windows, the file's own folder is the working directory of the app that opens it.",
    ],
    examples: [
      {
        title: "Open a file the user just exported",
        lang: "ts",
        code: `import { appShell, type AppDesktopError } from "@skathi-native/runtime";

export async function openExport(path: string): Promise<string | null> {
  try {
    await appShell.openPath(path);
    return null;
  } catch (e) {
    const error = e as AppDesktopError;
    if (error.code === "ENOASSOC") return "No app on this computer opens this kind of file.";
    if (error.code === "ENOENT") return "The file was moved or deleted.";
    return error.message;
  }
}`,
      },
    ],
    edgeCases: [
      'A refused program rejects with an `Error` named `NotAllowedError` and `code: "EEXECUTABLE"`.',
      PATH_RULES,
      "On Windows, a file type with no associated app rejects with `NotFoundError` (`ENOASSOC`).",
    ],
    limitations: [MAC, LINUX_TOOLS],
    related: ["shell-show-item-in-folder", "shell-open-external", "storage-get-user-folder", "app-desktop-error"],
    aliases: ["shell.openPath", "open file", "launch file"],
  },
  {
    slug: "shell-show-item-in-folder",
    name: "appShell.showItemInFolder()",
    kind: "method",
    summary: "Opens the folder that contains a file in the file manager, with the file selected.",
    platforms: PLATFORMS,
    syntax: "appShell.showItemInFolder(path: string): Promise<void>",
    params: [{ name: "path", type: "string", description: "An absolute path to a file or folder." }],
    returns: "A promise that resolves once the file manager has been asked.",
    notes: [
      "On Windows, Explorer opens the parent folder with the item selected; a drive root opens itself.",
      "On Linux, the file manager is asked over D-Bus (`org.freedesktop.FileManager1`) to select the item. Without one that answers, the parent folder opens with `xdg-open`, nothing selected.",
    ],
    examples: [
      {
        title: "Reveal a download",
        lang: "ts",
        code: `import { appShell } from "@skathi-native/runtime";

export function reveal(path: string): void {
  appShell.showItemInFolder(path).catch((error) => console.warn("Cannot show", path, error));
}`,
      },
    ],
    edgeCases: [PATH_RULES],
    limitations: [MAC, LINUX_TOOLS],
    related: ["shell-open-path", "shell-trash-item"],
    aliases: ["shell.showItemInFolder", "reveal in Explorer", "show in folder", "reveal in Finder"],
  },
  {
    slug: "shell-trash-item",
    name: "appShell.trashItem()",
    kind: "method",
    summary: "Moves a file or folder to the Recycle Bin or the trash. It never deletes permanently.",
    platforms: PLATFORMS,
    syntax: "appShell.trashItem(path: string): Promise<void>",
    params: [{ name: "path", type: "string", description: "An absolute path to a file or folder." }],
    returns: "A promise that resolves once the item is in the trash.",
    notes: [
      "On Windows, the move shows no dialog and can be undone from Explorer like any other delete.",
      "On Linux, it runs `gio trash`.",
    ],
    examples: [
      {
        title: "Delete an attachment with a way back",
        lang: "ts",
        code: `import { appShell, type AppDesktopError } from "@skathi-native/runtime";

export async function removeAttachment(path: string): Promise<"trashed" | "kept"> {
  try {
    await appShell.trashItem(path);
    return "trashed";
  } catch (e) {
    if ((e as AppDesktopError).code === "ENOTRASH") console.warn("This drive has no Recycle Bin; nothing was deleted.");
    return "kept";
  }
}`,
      },
    ],
    edgeCases: [
      'On Windows, a drive without a Recycle Bin, or an item too large for it, rejects with an `Error` named `InvalidStateError` and `code: "ENOTRASH"`; nothing is deleted.',
      "A file another program holds open rejects with `NoModificationAllowedError` (`EBUSY`) on Windows.",
      PATH_RULES,
    ],
    limitations: [MAC, LINUX_TOOLS],
    related: ["shell-show-item-in-folder", "app-desktop-error"],
    aliases: ["shell.trashItem", "recycle bin", "move to trash", "delete file"],
  },
  {
    slug: "app-desktop-error",
    name: "AppDesktopError",
    kind: "interface",
    summary:
      "The rejection of an `appShell` call: an `Error` whose `name` says what kind of failure, with a Node-style `code` and the raw OS error.",
    platforms: all(OK),
    syntax: `interface AppDesktopError extends Error {
  name: string; // "NotFoundError", "NotAllowedError", …
  code?: string; // "ENOENT", "EEXECUTABLE", …
  osError?: number;
}`,
    values: [
      { value: "ENOENT", status: OK, note: "`NotFoundError`: the file or folder does not exist." },
      {
        value: "ENOASSOC",
        status: OK,
        note: "`NotFoundError`, Windows: no app is registered for the file type or URL scheme.",
      },
      { value: "EEXECUTABLE", status: OK, note: "`NotAllowedError`: `openPath()` refused a program or script." },
      { value: "EACCES", status: OK, note: "`NotAllowedError`: access is denied." },
      { value: "EBUSY", status: OK, note: "`NoModificationAllowedError`, Windows: another program uses the file." },
      { value: "ENOTRASH", status: OK, note: "`InvalidStateError`, Windows: the item cannot go to the Recycle Bin." },
      { value: "ECANCELED", status: OK, note: "`AbortError`, Windows: the operation was cancelled." },
      {
        value: "ENOSYS",
        status: OK,
        note: "`NotSupportedError`: not available on this platform, or a Linux tool is missing.",
      },
      { value: "EIO", status: OK, note: "`InvalidStateError`: any other OS failure; the message carries the OS text." },
    ],
    notes: [
      "It is a plain `Error`, not a `DOMException`: test `error.code` or `error.name`, not `instanceof DOMException`.",
      "A bad argument is a `TypeError` without `code`.",
      "On Linux, a tool that exits with an error rejects with `OperationError` and no `code`.",
      "`osError` is the Win32 error code (or the HRESULT) on Windows, the `errno` from the file check elsewhere.",
    ],
    examples: [
      {
        title: "One message per failure",
        lang: "ts",
        code: `import type { AppDesktopError } from "@skathi-native/runtime";

export function describeShellError(e: unknown): string {
  if (e instanceof TypeError) return "That path or URL is not valid.";
  const error = e as AppDesktopError;
  switch (error.code) {
    case "ENOENT":
      return "It no longer exists.";
    case "EACCES":
      return "Access is denied.";
    case "EEXECUTABLE":
      return "Programs are not opened from here.";
    default:
      return error.message;
  }
}`,
      },
    ],
    related: ["app-shell", "shell-open-path", "shell-trash-item"],
    aliases: ["error code", "ENOENT"],
  },
]);
