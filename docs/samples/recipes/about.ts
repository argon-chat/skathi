import { app } from "skathi:app";
import { os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";

export function aboutLines(): string[] {
  return [
    `Version ${app.getVersion()}`,
    `Skathi ${runtime.version}${Runtime.isAot() ? ", AOT" : ""}`,
    `${os.platform} ${os.release}, ${os.arch}`,
  ];
}
