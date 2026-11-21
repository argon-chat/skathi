import { dlopen, dlopen as open } from "skathi:ffi";
import * as ffi from "skathi:ffi";

declare const picked: string;
declare const either: "crt" | "zlib";
const fixed = "crt";

export const libraries = [
  dlopen("crt", {}),
  dlopen("curl", {}),
  open(picked, {}),
  ffi.dlopen(either, {}),
  dlopen(fixed, {}),
];

export function shadowed(dlopen: (name: string) => unknown) {
  return dlopen("anything");
}
