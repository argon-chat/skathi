import { isSkBank, readSkBank } from "./skbank";
import { readSkpk } from "./skpk";

export interface PayloadEntry {
  kind: number;
  name: string;
  start: number;
  len: number;
  alignLog2?: number;
}

export interface Payload {
  entries: PayloadEntry[];
  index: Uint8Array;
  dataOffset: number;
  bytes(entry: PayloadEntry): Uint8Array;
  find(kind: number, name: string): PayloadEntry | undefined;
}

/** Open the .skpk emitted for either a current or a legacy workload. */
export function readPayload(blob: Uint8Array): Payload {
  if (isSkBank(blob)) return readSkBank(blob);
  const pack = readSkpk(blob);
  return {
    entries: pack.entries,
    index: blob.subarray(0, pack.header.namesOff + pack.header.namesLen),
    dataOffset: pack.header.dataOff,
    bytes: (entry) => blob.subarray(entry.start, entry.start + entry.len),
    find: pack.find,
  };
}
