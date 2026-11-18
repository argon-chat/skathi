---
title: Links and downloads
label: Links
metaTitle: "<a href> and <a download> in Skathi"
description: What a link does in an app without a browser. <a href> opens web and mail links in the system, relative links do nothing, and <a download> saves blob and data URLs through a Save As dialog.
section: Elements
order: 13
---

A Skathi app has no document to navigate: the page is your app. So `<a href>` does not load a page. It hands web and mail links to the system, and leaves in-app navigation to your router.

## `<a href>`

| `href`                       | A click                                                                                        |
| ---------------------------- | ---------------------------------------------------------------------------------------------- |
| `https:`, `http:`            | Opens the URL in the system's default browser.                                                 |
| `mailto:`                    | Opens the system's mail client.                                                                |
| A relative path, `?query`    | Does nothing, with an advisory.                                                                |
| `#` or `#fragment`, or empty | Does nothing.                                                                                  |
| `javascript:`                | Never runs; anything but `javascript:void(0)` prints an advisory.                              |
| Any other scheme             | Refused, with an advisory that names [`appShell.openExternal`](/app-api/shell-open-external/). |

The link opens on a click or on Enter when it has focus, after the click's listeners have run. `preventDefault()` in a listener cancels it, as in browsers. `target` makes no difference: there is no other tab or frame to open in.

For in-app navigation use your router's link, such as Vue Router's `<RouterLink>`, which handles the click itself. For a URL your own code decides to open, such as a `file:` path or a custom scheme the app trusts, call [`appShell.openExternal(url)`](/app-api/shell-open-external/). Links are limited to `http`, `https` and `mailto` because their URLs can come from content.

`window.open(url)` with an `http`, `https` or `mailto` URL does the same as a link and returns `null`. For a second window of your own app, use [`<SkWindow>`](/elements/sk-window/).

## `<a download>`

A link with a `download` attribute whose `href` is a `blob:` or `data:` URL saves the file:

```ts
export function downloadCsv(rows: string[][], name = "report.csv") {
  const blob = new Blob([rows.map((r) => r.join(",")).join("\n")], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}
```

- The file is saved through the system's Save As dialog, which starts in the Downloads folder with the `download` name filled in. Chromium saves to Downloads without asking.
- The URL is read at the click, so revoking it right after `click()` still saves the file, and the link never has to join the tree.
- Only `blob:` and `data:` URLs are saved. An `https:` link with `download` opens in the system browser like any link.

The dialog is `IFileSaveDialog` on Windows, `NSSavePanel` on macOS and the XDG desktop portal on Linux. [`showSaveFilePicker()`](/web-platform/file-system-access/) gives you a handle to write to instead, for a file you write in parts or more than once.

## See also

- [`<a download>` in the web-platform reference](/web-platform/a-download/).
- [`appShell`](/app-api/app-shell/): opening URLs, paths and folders from code.
- [Form controls](/elements/form-controls/#file): choosing files to open.
