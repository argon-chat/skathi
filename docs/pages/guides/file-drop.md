---
title: Drop files into a window
description: Accept files from a desktop file manager, preview large videos, read dropped directories and pass paths to native code.
section: Guides
order: 13
---

# Drop files into a window

A Skathi window receives files from the system file manager through HTML drag events (since 0.6).
Call `preventDefault()` on `dragover` to accept a drop. Set `dropEffect` to an operation the source offers to select the cursor. Use `copy` when opening or previewing a file. A source can remove the original after a successful `move`.

## Preview video

This Vue component keeps its highlight while the pointer moves between children of the drop zone. It releases the previous object URL when the user opens another file and when the component unmounts.

```vue
<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";

const depth = ref(0);
const src = ref("");

function over(e: DragEvent) {
  if (!e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
}
function drop(e: DragEvent) {
  depth.value = 0;
  const file = e.dataTransfer?.files[0];
  if (!file?.type.startsWith("video/")) return;
  e.preventDefault();
  if (src.value) URL.revokeObjectURL(src.value);
  src.value = URL.createObjectURL(file);
}
onBeforeUnmount(() => {
  if (src.value) URL.revokeObjectURL(src.value);
});
</script>

<template>
  <section :class="{ over: depth > 0 }" @dragenter="depth++" @dragleave="depth--" @dragover="over" @drop="drop">
    <video v-if="src" :src="src" controls />
    <p v-else>Drop a video</p>
  </section>
</template>
<style>
.over {
  outline: 2px solid #3575cc;
}
</style>
```

A Svelte drop zone uses the same events.

```svelte
<script lang="ts">
  let names = $state<string[]>([]);
  let depth = $state(0);
  function over(e: DragEvent) {
    if (e.dataTransfer?.types.includes("Files")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    }
  }
  function drop(e: DragEvent) {
    e.preventDefault();
    depth = 0;
    names = Array.from(e.dataTransfer?.files ?? [], (file) => file.name);
  }
</script>

<section class:over={depth > 0} ondragenter={() => depth++} ondragleave={() => depth--} ondragover={over} ondrop={drop}>
  {names.join(", ") || "Drop files"}
</section>

<style>
  .over {
    outline: 2px solid #3575cc;
  }
</style>
```

## Choose how to access a drop

| You need                   | API                                            | Work performed                                                                                                                               |
| -------------------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| MIME types while hovering  | `event.dataTransfer.items`                     | Types come from extensions. File contents and paths become readable in the drop listener.                                                    |
| A native path              | `appFiles.getPathForFile(fileOrEntryOrHandle)` | Returns the absolute disk path. A `new File(...)` or a virtual filesystem root yields `null`.                                                |
| A media preview            | `URL.createObjectURL(file)`                    | A whole disk-backed file opens directly in the native media loader. Revoke the URL after use.                                                |
| Part of a file             | `file.slice(start, end).arrayBuffer()`         | Reads only the requested byte range.                                                                                                         |
| A sequential read          | `file.stream().getReader()`                    | Reads disk blocks of up to 1 MiB as needed and delivers chunks of up to 64 KiB. Cancelling releases the buffered block.                      |
| All bytes at once          | `file.arrayBuffer()`, `file.text()`            | Allocates the selected range in memory.                                                                                                      |
| A read-only directory walk | `item.webkitGetAsEntry()`                      | Each reader returns at most 100 entries per call. An empty batch marks the end.                                                              |
| Read or write access       | `await item.getAsFileSystemHandle?.()`         | A file handle exposes `getFile()` and `createWritable()`. Close the writable to commit changes. A directory handle supports async iteration. |
| A Rust module argument     | `skathi::HostFile`                             | A native module receives the host-created DOM `File` as a `PathBuf` in its `.0` field.                                                       |

Use `import { appFiles } from "skathi:app"`. Save a `File`, entry or handle promise during the drop listener. A retained `DataTransferItem` becomes unavailable after the synchronous event dispatch.

```ts
async function walk(entry: FileSystemEntry, visit: (file: File) => void) {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
    visit(file);
    return;
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
    if (!batch.length) return;
    for (const child of batch) await walk(child, visit);
  }
}
```

A dropped directory has a zero-length placeholder in `files`. Use its entry or handle for the children. An ordinary drop leaves `File.webkitRelativePath` empty. Each entry has a virtual `fullPath` relative to the root of the drop.

An enabled `<input type="file">` accepts drops by default and fires `input` followed by `change`. A single-file input takes the first file. The picker filter in `accept` does not restrict dropped files. Other elements require an accepting `dragover` listener. Dropping on an unhandled area leaves the current document open. Each native window targets the elements painted in its own client area. Embedded iframe documents receive the event on their iframe element.

Devtools can deliver a gesture through the same input path. Relative to the target window, `drag-files --paths /absolute/path/to/clip.mp4 --at 120,80` performs a full drop. Use `drag-files enter`, `over`, `leave` or `drop` to test each stage, with `--ctrl`, `--alt`, `--shift` or `--meta` as needed.

## Drag files out of a window (since 0.6)

To try both directions, run `bun run showcase` in the Skathi repository and open **Desktop → Files**. Drop files from a folder, then drag the group to another folder. Drag **notes.txt** out to export the current editor text.

Mark the source as `draggable` and fill its `DataTransfer` in `dragstart`. A pointer move of four CSS pixels starts a system drag. A destination such as Explorer or Finder can copy an existing file or request the contents of a newly generated file.

```ts
const source = document.querySelector("#export")!;
source.setAttribute("draggable", "true");
source.addEventListener("dragstart", (event) => {
  const store = (event as DragEvent).dataTransfer!;
  store.effectAllowed = "copy";
  store.items.add(new File(["Hello\n"], "note.txt", { type: "text/plain" }));
  store.setData("text/plain", "Hello");
  store.setDragImage(source, 12, 8);
});
```

A host-created file from a drop or file picker uses its original path. For a generated file, Windows provides indexed OLE streams, macOS provides an AppKit file promise and Linux materializes private temporary files when the receiver requests their URIs. Use `DownloadURL` with `mime:filename:url` to export a blob URL, a data URL or an application asset. For a remote resource, fetch it in your app before dragging and supply the resulting `File`.

Other applications can receive `text/plain`, `text/uri-list` and `text/html`. Windows of the same Skathi application can also exchange custom MIME types and the original `File` objects. During `dragover`, inspect `types` or `items`. Read the values in `drop`.

The source receives periodic `drag` events. In `dragend`, read `event.dataTransfer.dropEffect` to learn the destination's final operation. Escape and rejected drops yield `none`. X11, Windows and macOS can negotiate links. Wayland negotiates copy and move. Windows, macOS and Wayland display a snapshot of the visible source or the element supplied to `setDragImage`. X11 uses its native pointer.

To test a transfer between two application windows, run `drag-document arm`, send a mouse press and a move with the left button held, then send `drag-document enter`, `over` and `drop` to the target selector. `drag-document cancel` ends the source session with `none`.
