// Stamping: a copy of a host exe becomes ONE app's exe. Copy host -> <out>.partial, strip any
// certificate table (the author signs the stamped exe later), write icons, VERSIONINFO and the
// VNPAYLOAD resource (pass 1 with skew 0, pass 2 with the skew that page-aligns the VNPK data
// region in the mapped image), set the subsystem to GUI unless asked to keep the console, verify
// by mapping it as an image resource, rename over <out>. The file is freshly written, which is
// when Windows Defender likes to scan it, so the resource transactions and the rename retry on a
// sharing violation.

import { copyFileSync, existsSync, renameSync, rmSync, statSync, truncateSync } from "node:fs";
import { groupIcon, type IconImage } from "./icon";
import {
  ERROR_LOCK_VIOLATION,
  ERROR_SHARING_VIOLATION,
  loadResourceCopy,
  patchFile,
  readFileRange,
  readPe,
  ResourceUpdate,
  RT,
  setSubsystem,
  shellIconCount,
  SUBSYSTEM,
  Win32Error,
  type PeInfo,
  type ResourceEntry,
} from "./pe";
import { HEADER_SIZE, PAGE, readHeader, skewFor } from "./vnpk";

export const PAYLOAD_NAME = "VNPAYLOAD";
export const RES_LANG = 0x0409;
export const PAYLOAD_LANG = 0;

export interface StampOptions {
  host: string;
  out: string;
  payload: (skew: number) => Uint8Array;
  icons?: IconImage[];
  versionInfo?: Uint8Array;
  subsystem?: "gui" | "console" | "keep";
  log?: (line: string) => void;
}

export interface StampResult {
  out: string;
  bytes: number;
  payloadBytes: number;
  payloadRva: number;
  skew: number;
  dataAligned: boolean;
  passes: number;
  strippedSignature: boolean;
  subsystem: number;
  iconCount: number | null;
  ms: { copy: number; resources: number; verify: number; total: number };
  payload: Uint8Array;
}

const sleep = (ms: number) => Bun.sleepSync(ms);

function isSharing(e: unknown): boolean {
  if (e instanceof Win32Error) return e.code === ERROR_SHARING_VIOLATION || e.code === ERROR_LOCK_VIOLATION;
  const code = (e as NodeJS.ErrnoException)?.code;
  return code === "EBUSY" || code === "EPERM" || code === "EACCES";
}

function retrying<T>(what: string, f: () => T, log: (l: string) => void, attempts = 8): T {
  let delay = 100;
  for (let i = 1; ; i++) {
    try {
      return f();
    } catch (e) {
      if (!isSharing(e) || i >= attempts) throw e;
      log(`[vn] ${what}: the file is busy (${(e as Error).message}) — retrying in ${delay} ms`);
      sleep(delay);
      delay = Math.min(delay * 2, 2000);
    }
  }
}

function payloadEntry(pe: PeInfo): ResourceEntry | undefined {
  return pe.resources.find((r) => r.type === RT.RCDATA && r.name === PAYLOAD_NAME);
}

/** Removes an Authenticode certificate table (truncate when it is the file's tail, zero the SECURITY entry either way). Returns whether there was one. */
export function stripSignature(path: string): boolean {
  const pe = readPe(path);
  if (!pe.security.size) return false;
  const size = statSync(path).size;
  if (pe.security.offset > 0 && pe.security.offset + pe.security.size >= size - 8) {
    truncateSync(path, pe.security.offset);
  }
  patchFile(path, pe.securityEntryOffset, new Uint8Array(8));
  return true;
}

export function stamp(options: StampOptions): StampResult {
  const log = options.log ?? (() => {});
  const t0 = performance.now();
  const partial = `${options.out}.partial`;
  rmSync(partial, { force: true });
  retrying("copying the host", () => copyFileSync(options.host, partial), log);
  const tCopy = performance.now();

  let strippedSignature = false;
  let result: StampResult | null = null;
  try {
    strippedSignature = stripSignature(partial);
    if (strippedSignature) log("[vn] removed the host's certificate table — sign the stamped exe instead");

    const before = readPe(partial);
    let skew = 0;
    let blob = options.payload(skew);
    let passes = 1;

    retrying(
      "stamping resources",
      () => {
        const u = new ResourceUpdate(partial);
        try {
          for (const r of before.resources) {
            const lang = r.lang;
            if (r.type === RT.RCDATA && r.name === PAYLOAD_NAME) u.remove(r.type, r.name, lang);
            if (options.icons && (r.type === RT.ICON || r.type === RT.GROUP_ICON)) u.remove(r.type, r.name, lang);
            if (options.versionInfo && r.type === RT.VERSION) u.remove(r.type, r.name, lang);
          }
          if (options.icons?.length) {
            const { icons, group } = groupIcon(options.icons, 1);
            for (const icon of icons) u.set(RT.ICON, icon.id, RES_LANG, icon.data);
            u.set(RT.GROUP_ICON, 1, RES_LANG, group);
          }
          if (options.versionInfo) u.set(RT.VERSION, 1, RES_LANG, options.versionInfo);
          u.set(RT.RCDATA, PAYLOAD_NAME, PAYLOAD_LANG, blob);
          u.commit();
        } catch (e) {
          u.discard();
          throw e;
        }
      },
      log,
    );

    // Passes 2..3: the skew. The RVA is decided by what sorts ahead of RT_RCDATA (the RT_ICONs).
    let pe = readPe(partial);
    let entry = payloadEntry(pe);
    if (!entry) throw new Error(`${partial}: the payload resource is missing after EndUpdateResourceW`);
    const header0 = readHeader(blob);
    for (;;) {
      const want = skewFor(entry.rva, header0);
      const dataOff = header0.namesOff + header0.namesLen + skew;
      if ((entry.rva + dataOff) % PAGE === 0 || passes >= 3) break;
      skew = want;
      blob = options.payload(skew);
      passes++;
      retrying(
        "re-stamping the payload",
        () => {
          const u = new ResourceUpdate(partial);
          try {
            u.set(RT.RCDATA, PAYLOAD_NAME, PAYLOAD_LANG, blob);
            u.commit();
          } catch (e) {
            u.discard();
            throw e;
          }
        },
        log,
      );
      pe = readPe(partial);
      entry = payloadEntry(pe);
      if (!entry) throw new Error(`${partial}: the payload resource vanished in pass ${passes}`);
    }
    const header = readHeader(blob);
    const dataAligned = (entry.rva + header.dataOff) % PAGE === 0;
    if (!dataAligned) {
      log(
        `[vn] warning: the payload's data region did not land on a page boundary (rva 0x${entry.rva.toString(16)}, ` +
          `data_off ${header.dataOff}) — the app runs exactly the same; only a future bytecode cache cares`,
      );
    }

    if (options.subsystem !== "keep") {
      setSubsystem(partial, options.subsystem === "console" ? SUBSYSTEM.CONSOLE : SUBSYSTEM.GUI);
    }
    const tRes = performance.now();

    // Verify the way the host reads it, so a file Windows will not map fails here, not on a user's machine.
    pe = readPe(partial);
    entry = payloadEntry(pe);
    if (!entry || entry.size !== blob.length) {
      throw new Error(`verify: the stamped payload is ${entry?.size ?? "missing"} bytes, expected ${blob.length}`);
    }
    const mapped = loadResourceCopy(partial, RT.RCDATA, PAYLOAD_NAME, HEADER_SIZE);
    if (!mapped || !bytesEqual(mapped, blob.subarray(0, HEADER_SIZE))) {
      throw new Error("verify: FindResourceW on the stamped image did not return the payload that was written");
    }
    const onDisk = readFileRange(partial, entry.fileOffset, Math.min(blob.length, 4096));
    if (!bytesEqual(onDisk, blob.subarray(0, onDisk.length))) {
      throw new Error("verify: the payload bytes in the file differ from the blob that was written");
    }

    retrying(
      `replacing ${options.out}`,
      () => {
        if (existsSync(options.out)) {
          try {
            rmSync(options.out);
          } catch (e) {
            const err = new Error(
              `${options.out} cannot be replaced — is it running? Close it and build again. (${(e as Error).message})`,
            );
            (err as NodeJS.ErrnoException).code = (e as NodeJS.ErrnoException).code;
            throw err;
          }
        }
        renameSync(partial, options.out);
      },
      log,
      6,
    );
    const iconCount = options.icons?.length ? shellIconCount(options.out) : null;
    if (options.icons?.length && !iconCount) {
      throw new Error(`verify: the shell sees no icon in ${options.out} (ExtractIconExW returned 0)`);
    }
    const tVerify = performance.now();
    result = {
      out: options.out,
      bytes: statSync(options.out).size,
      payloadBytes: blob.length,
      payloadRva: entry.rva,
      skew,
      dataAligned,
      passes,
      strippedSignature,
      subsystem: pe.subsystem,
      iconCount,
      ms: {
        copy: Math.round(tCopy - t0),
        resources: Math.round(tRes - tCopy),
        verify: Math.round(tVerify - tRes),
        total: Math.round(tVerify - t0),
      },
      payload: blob,
    };
    return result;
  } finally {
    if (!result) rmSync(partial, { force: true });
  }
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}
