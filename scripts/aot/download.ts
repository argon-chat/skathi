// Acquisition of one immutable, reviewed build input. No package installation,
// postinstall script or executable runs here.
import { randomUUID } from "node:crypto";
import { existsSync, renameSync } from "node:fs";
import { open, unlink } from "node:fs/promises";
import { verifyFile } from "./toolchain";

export async function downloadVerified(
  url: string, path: string, expected: string,
  fetcher: typeof fetch = fetch, maximumBytes = 256 * 1024 * 1024,
): Promise<void> {
  if (existsSync(path)) { await verifyFile(path, expected); return; }
  const temporary = `${path}.download-${process.pid}-${randomUUID()}`;
  // If exclusive creation fails we own nothing, and must not remove that path.
  const handle = await open(temporary, "wx");
  let closed = false, ownsTemporary = true;
  let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
  try {
    const response = await fetcher(url, { headers: { "User-Agent": "vue-native-native-aot-build" } });
    if (!response.ok || !response.body) throw Error(`Pinned native-AOT frontend download failed (${response.status}); no moving canary fallback is allowed. Supply a hash-verified --frontend or seed the build-tool cache.`);
    reader = response.body.getReader();
    const declared = Number(response.headers.get("content-length"));
    if (Number.isFinite(declared) && declared > maximumBytes) throw Error("Pinned frontend archive exceeds the download bound");
    let received = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      received += value.byteLength;
      if (received > maximumBytes) throw Error("Pinned frontend archive exceeds the download bound");
      for (let offset = 0; offset < value.byteLength;) {
        const { bytesWritten } = await handle.write(value, offset, value.byteLength - offset);
        if (bytesWritten <= 0) throw Error("Native frontend staging write made no progress");
        offset += bytesWritten;
      }
    }
    await handle.sync(); await handle.close(); closed = true;
    await verifyFile(temporary, expected);
    try { renameSync(temporary, path); ownsTemporary = false; }
    catch (error) {
      // A correct concurrent publisher is reusable; a differing archive is an
      // error. Never publish through or overwrite an open destination in place.
      try { await verifyFile(path, expected); } catch { throw error; }
    }
  } finally {
    if (reader) { await reader.cancel().catch(() => {}); reader.releaseLock(); }
    if (!closed) await handle.close().catch(() => {});
    if (ownsTemporary) await unlink(temporary).catch(error => {
      if (error?.code !== "ENOENT") throw error;
    });
  }
}
