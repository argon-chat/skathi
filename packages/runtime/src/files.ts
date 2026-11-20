/** Files, directory entries and handles granted by the desktop host (since 0.6). */
export interface AppFiles {
  /** Absolute disk path. Script-created files and the virtual root have no path. */
  getPathForFile(fileOrEntryOrHandle: object): string | null;
}

declare const __sk_appFiles: AppFiles;
export const appFiles: AppFiles = {
  getPathForFile(fileOrEntryOrHandle) {
    return __sk_appFiles.getPathForFile(fileOrEntryOrHandle);
  },
};
