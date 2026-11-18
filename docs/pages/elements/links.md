---
title: "<a>"
label: "<a>"
metaTitle: "<a href> and <a download> in Skathi"
description: What a link does in an app without a browser. <a href> opens web and mail links in the system, relative links do nothing, and <a download> saves blob and data URLs through a Save As dialog.
section: Elements
order: 13
---

Apps link out all the time: to documentation, a changelog, a support address, a pricing page. They also hand the user files: an exported report, a generated image, a backup. A Skathi app has no document to navigate, because the page is your app, so `<a>` covers exactly those two jobs. `<a href>` gives web and mail links to the system, which opens them in the user's own browser or mail client, and `<a download>` saves a file you made through the system's Save As dialog. Navigation inside the app stays with your router.

## Link to the web

A plain `<a href>` with an `https:` or `mailto:` URL is all it takes. The click opens the system's default browser or mail client after your click listeners have run, and `preventDefault()` in a listener cancels it, as in a browser.

```vue title="About.vue" framework="vue"
<template>
  <p>
    Read the <a href="https://example.com/docs">documentation</a> or write to
    <a href="mailto:support@example.com">support</a>.
  </p>
</template>
```

```svelte title="About.svelte" framework="svelte"
<p>
  Read the <a href="https://example.com/docs">documentation</a> or write to
  <a href="mailto:support@example.com">support</a>.
</p>
```

`target` makes no difference: there is no other tab or frame to open in. For links inside the app, use your router's link, such as Vue Router's `<RouterLink>`, which handles the click itself. A relative `href` does nothing, with an advisory.

## Save a file you generated

A link with a `download` attribute and a `blob:` or `data:` URL saves the file. The user picks where, in the system's Save As dialog, which starts in the Downloads folder with the `download` name filled in. The link never has to join the tree, so an export button can create one, click it and forget it:

```vue title="ExportButton.vue" framework="vue"
<script setup lang="ts">
const props = defineProps<{ rows: string[][] }>();

function exportCsv() {
  const blob = new Blob([props.rows.map((r) => r.join(",")).join("\n")], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "report.csv";
  link.click();
  URL.revokeObjectURL(url);
}
</script>

<template>
  <button @click="exportCsv">Export CSV</button>
</template>
```

```svelte title="ExportButton.svelte" framework="svelte"
<script lang="ts">
  let { rows }: { rows: string[][] } = $props();

  function exportCsv() {
    const blob = new Blob([rows.map((r) => r.join(",")).join("\n")], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "report.csv";
    link.click();
    URL.revokeObjectURL(url);
  }
</script>

<button onclick={exportCsv}>Export CSV</button>
```

The URL is read at the click, so revoking it right after `click()` still saves the file. Where Chromium saves to Downloads without asking, Skathi always shows the dialog. [`showSaveFilePicker()`](/web-platform/file-system-access/) gives you a handle to write to instead, for a file you write in parts or more than once.

## Things to watch for

- **Only `http`, `https` and `mailto` open.** Other schemes are refused with an advisory, because a link's URL can come from content. For a `file:` path or a custom scheme your own code trusts, call [`appShell.openExternal(url)`](/app-api/shell-open-external/).
- **`download` saves only `blob:` and `data:`.** An `https:` link with `download` opens in the system browser like any link.
- **`javascript:` never runs.** Anything but `javascript:void(0)` prints an advisory.
- **`window.open(url)`** with an `http`, `https` or `mailto` URL does the same as a link and returns `null`. For a second window of your own app, use [`SkWindow`](/elements/sk-window/).

## `<a href>`

| `href`                       | A click                                                                                        |
| ---------------------------- | ---------------------------------------------------------------------------------------------- |
| `https:`, `http:`            | Opens the URL in the system's default browser.                                                 |
| `mailto:`                    | Opens the system's mail client.                                                                |
| A relative path, `?query`    | Does nothing, with an advisory.                                                                |
| `#` or `#fragment`, or empty | Does nothing.                                                                                  |
| `javascript:`                | Never runs; anything but `javascript:void(0)` prints an advisory.                              |
| Any other scheme             | Refused, with an advisory that names [`appShell.openExternal`](/app-api/shell-open-external/). |

The link opens on a click or on Enter when it has focus, after the click's listeners have run.

## `<a download>`

- The file is saved through the system's Save As dialog, which starts in the Downloads folder with the `download` name filled in.
- The URL is read at the click, so revoking it right after `click()` still saves the file, and the link never has to join the tree.
- Only `blob:` and `data:` URLs are saved.

The dialog is `IFileSaveDialog` on Windows, `NSSavePanel` on macOS and the XDG desktop portal on Linux.

## See also

- [`<a download>` in the web-platform reference](/web-platform/a-download/).
- [`appShell`](/app-api/app-shell/): opening URLs, paths and folders from code.
- [Form controls](/elements/form-controls/#file): choosing files to open.
