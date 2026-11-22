import { Process } from "skathi:process";
import * as processes from "skathi:process";

declare const picked: string;

export const started = [
  Process.spawn("git", ["status"]),
  Process.spawn("curl"),
  processes.Process.exec(picked),
  Process.spawn("C:/tools/tool.exe"),
];
