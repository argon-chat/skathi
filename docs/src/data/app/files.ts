import { OK, all, importLine, section } from "./shared";

export const files = section("Files", [
  {
    slug: "app-files",
    name: "appFiles",
    kind: "property",
    summary: "Returns the disk path of a dropped or picked `File`, a directory entry or a file system handle.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: `${importLine("appFiles")}

appFiles.getPathForFile(fileOrEntryOrHandle: File | FileSystemEntry | FileSystemHandle): string | null`,
    returns: "The absolute path on disk, or `null` when the object has none.",
    notes: [
      "A `File` from a drop or a file picker, an entry from `webkitGetAsEntry()` and a handle the user granted all carry their path. See [Drop files into a window](/guides/file-drop/).",
      "Like Electron's `webUtils.getPathForFile()`.",
    ],
    examples: [
      {
        title: "Paths of dropped files",
        lang: "ts",
        code: `import { appFiles } from "skathi:app";

addEventListener("drop", (event) => {
  for (const file of event.dataTransfer?.files ?? []) {
    console.log(appFiles.getPathForFile(file));
  }
});`,
      },
    ],
    edgeCases: [
      "A `File` made in script (`new File(...)`), the root of a virtual file system and a handle that was not granted give `null`.",
    ],
    related: ["app-shell", "storage-get-user-folder"],
    aliases: ["getPathForFile", "webUtils.getPathForFile", "file path", "native path"],
  },
]);
