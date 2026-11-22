import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export function signal(state: string) {
  void getCurrentWindow().setTitle(`Compare ${state}`);
}
export const benchMode = () => invoke<boolean>("bench");
export const add = (a: number, b: number) => invoke<number>("add", { a, b });
export const len = (bytes: Uint8Array) => invoke<number>("len", bytes);
