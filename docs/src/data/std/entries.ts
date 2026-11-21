import { app } from "./app";
import { binary } from "./binary";
import { collections } from "./collections";
import { encoding } from "./encoding";
import { ffi } from "./ffi";
import { network } from "./network";
import { numeric } from "./numeric";
import { os } from "./os";
import { runtime } from "./runtime";
import { categories, type Category, type StdEntry } from "./shared";
import { time } from "./time";
import { toml } from "./toml";
import { units } from "./units";

export { categories };
export type { Category, StdEntry };

export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {
  "skathi:app":
    "Ending and restarting the app and reading its version. The same module exports the `app*` objects (tray, hotkeys, storage and the rest), which have their own reference: [App APIs](/app-api/).",
  "skathi:runtime":
    "The Skathi the app runs on: its version, the features the host was built with, and whether the code was compiled ahead of time or packed into a single file.",
  "skathi:os":
    "The machine: platform, architecture, OS release, host name, and the home and temp folders. Every value is read once and does not change while the app runs.",
  "skathi:network":
    "Whether there is a connection, whether it reaches the internet, what kind it is, and whether it is metered or roaming, with an event when any of that changes.",
  "skathi:numeric":
    "Fixed-width integers and floats with the arithmetic of the width: wrapping, checked and saturating operations, bit operations and exact conversions.",
  "skathi:toml": "TOML 1.0 to JS values and back.",
  "skathi:binary":
    "Reading and writing binary formats: byte and bit cursors over a buffer, byte order, LEB128 integers, and CRC-32 and CRC-64 checksums.",
  "skathi:encoding":
    "Bytes to text and back: hex, Base32, Base64, the legacy text encodings `TextDecoder` knows and can also encode here, and MIME types by extension or content.",
  "skathi:ffi":
    "C functions of a library the app ships or the system has, called from JS without a Rust build. Only the libraries sk.config.json lists under `permissions.ffi` load. A fault in the library ends the app; for your own native code, write a [native module](/embedding/native-modules/).",
  "skathi:time":
    "Durations as values, measuring elapsed time, deadlines that travel with a call, retry delays, rate limiting and a timer that does not drift.",
  "skathi:units":
    "Quantities that carry their unit: sizes of data, frequencies and angles, with parsing, conversion and formatting.",
  "skathi:collections":
    "Data structures that JS does not have and that are slow to write in it: bit sets, Bloom filters, ring buffers, disjoint sets, tries and interval trees.",
};

export const entries: readonly StdEntry[] = [
  ...app,
  ...runtime,
  ...os,
  ...network,
  ...numeric,
  ...toml,
  ...binary,
  ...encoding,
  ...ffi,
  ...time,
  ...units,
  ...collections,
];
