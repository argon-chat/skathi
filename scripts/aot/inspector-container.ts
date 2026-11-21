import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { appendMachOImage } from "../../packages/cli/cli/image-macho";
import { appendImageSections, INSPECTOR_SECTIONS, withUnwindTable } from "../../packages/cli/cli/image-sections";

export function writeInspectorContainer(path: string, template: Uint8Array, image: Uint8Array): Uint8Array {
  if (process.platform === "darwin") {
    writeFileSync(path, appendMachOImage(template, image));
    for (const args of [
      ["--force", "--sign", "-", "--timestamp=none", "--identifier", "org.skathi.inspector-image", path],
      ["--verify", "--strict", path],
    ]) {
      const result = spawnSync("codesign", args, { encoding: "utf8" });
      if (result.status !== 0) throw Error(`codesign ${args[0]} failed for ${path}: ${result.error ?? result.stderr}`);
    }
    return readFileSync(path);
  }
  const bytes = withUnwindTable(appendImageSections(template, image, INSPECTOR_SECTIONS), [INSPECTOR_SECTIONS]);
  writeFileSync(path, bytes);
  return bytes;
}
