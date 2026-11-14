// The Linux side of `sk build`: a copy of the host ELF with the payload appended.
//
//   <exeName> = host | zero pad to a page | SKPK | u64 offset | u64 length | "SKPKTAIL"
//
// The host maps /proc/self/exe and finds the payload by that trailer (`sk_payload::appended`). The
// offset is page-aligned and the SKPK skewed for offset 0, so its data region lands on a page
// boundary of the mapping. The file is written as `<out>.partial`, made executable and renamed.

import {
  chmodSync,
  closeSync,
  fstatSync,
  openSync,
  readFileSync,
  readSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { PAGE, readHeader, skewFor } from "./skpk";

export const TRAILER_MAGIC = "SKPKTAIL";
export const TRAILER_SIZE = 24;

const MACHINE: Record<number, string> = { 0x3e: "x86_64", 0xb7: "aarch64" };

/** The CPU of a 64-bit little-endian ELF, or null when `head` is not one. */
export function elfMachine(head: Uint8Array): string | null {
  if (head.length < 20 || head[0] !== 0x7f || head[1] !== 0x45 || head[2] !== 0x4c || head[3] !== 0x46) return null;
  if (head[4] !== 2 || head[5] !== 1) return null;
  const machine = head[18]! | (head[19]! << 8);
  return MACHINE[machine] ?? `machine 0x${machine.toString(16)}`;
}

function readAt(path: string, at: (size: number) => { pos: number; len: number }): Uint8Array | null {
  try {
    const fd = openSync(path, "r");
    try {
      const { pos, len } = at(fstatSync(fd).size);
      if (pos < 0 || len <= 0) return new Uint8Array(0);
      const buf = new Uint8Array(len);
      return buf.subarray(0, readSync(fd, buf, 0, len, pos));
    } finally {
      closeSync(fd);
    }
  } catch {
    return null;
  }
}

export function elfMachineOf(path: string): string | null {
  const head = readAt(path, () => ({ pos: 0, len: 20 }));
  return head && elfMachine(head);
}

export function trailer(offset: number, length: number): Uint8Array {
  const t = new Uint8Array(TRAILER_SIZE);
  const dv = new DataView(t.buffer);
  dv.setBigUint64(0, BigInt(offset), true);
  dv.setBigUint64(8, BigInt(length), true);
  t.set(new TextEncoder().encode(TRAILER_MAGIC), 16);
  return t;
}

/** Where the trailer at the end of `tail` says the payload is, in a file of `size` bytes; null without a valid one. */
export function appendedPayload(tail: Uint8Array, size: number): { offset: number; length: number } | null {
  if (tail.length < TRAILER_SIZE || size < TRAILER_SIZE) return null;
  const t = tail.subarray(tail.length - TRAILER_SIZE);
  if (new TextDecoder().decode(t.subarray(16)) !== TRAILER_MAGIC) return null;
  const dv = new DataView(t.buffer, t.byteOffset, TRAILER_SIZE);
  const offset = Number(dv.getBigUint64(0, true));
  const length = Number(dv.getBigUint64(8, true));
  return offset + length === size - TRAILER_SIZE ? { offset, length } : null;
}

export function appendedPayloadOf(path: string): { offset: number; length: number } | null {
  let size = 0;
  const tail = readAt(path, (s) => {
    size = s;
    return { pos: s - TRAILER_SIZE, len: TRAILER_SIZE };
  });
  return tail && appendedPayload(tail, size);
}

export interface ElfStampOptions {
  host: string;
  out: string;
  payload: (skew: number) => Uint8Array;
}

export interface ElfStampResult {
  out: string;
  bytes: number;
  payloadBytes: number;
  payloadOffset: number;
  skew: number;
  dataAligned: boolean;
  machine: string;
  ms: { total: number };
  payload: Uint8Array;
}

export function stampElf(options: ElfStampOptions): ElfStampResult {
  const t0 = performance.now();
  const host = new Uint8Array(readFileSync(options.host));
  const machine = elfMachine(host);
  if (!machine) throw new Error(`${options.host} is not a 64-bit ELF executable — a Linux build needs a Linux sk-app`);
  if (appendedPayload(host, host.length)) {
    throw new Error(`${options.host} is a packaged app, not a bare host (it already carries a payload)`);
  }
  const offset = Math.ceil(host.length / PAGE) * PAGE;
  const unskewed = options.payload(0);
  const skew = skewFor(0, readHeader(unskewed));
  const payload = skew ? options.payload(skew) : unskewed;
  const header = readHeader(payload);
  const file = new Uint8Array(offset + payload.length + TRAILER_SIZE);
  file.set(host);
  file.set(payload, offset);
  file.set(trailer(offset, payload.length), offset + payload.length);

  const partial = `${options.out}.partial`;
  try {
    writeFileSync(partial, file);
    chmodSync(partial, 0o755);
    rmSync(options.out, { force: true });
    renameSync(partial, options.out);
  } catch (e) {
    rmSync(partial, { force: true });
    throw new Error(`${options.out} cannot be written — is it running? (${(e as Error).message})`);
  }
  return {
    out: options.out,
    bytes: file.length,
    payloadBytes: payload.length,
    payloadOffset: offset,
    skew,
    dataAligned: (offset + header.dataOff) % PAGE === 0,
    machine,
    ms: { total: Math.round(performance.now() - t0) },
    payload,
  };
}
