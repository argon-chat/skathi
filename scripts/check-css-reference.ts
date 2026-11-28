import { deepStrictEqual } from "node:assert";
import { entries, type Status } from "../docs/src/data/css/entries";
import { nativeProperties, type NativeProperty } from "../docs/src/data/css/native";

const native: (NativeProperty & { platforms: string[]; advisory: string | null })[] = await Bun.stdin.json();
deepStrictEqual(
  [...nativeProperties.values()],
  native.map(({ platforms, advisory: _, ...property }) => {
    if (platforms.length) throw new Error(`Expose the native platforms for ${property.name} in the CSS reference`);
    return property;
  }),
  "The CSS reference must read the same declarations, support, samples, notes and aliases as sk-css.",
);
const reference = new Map(
  entries
    .filter((entry) => entry.kind === "property" || entry.kind === "shorthand")
    .map((entry) => [entry.name, entry]),
);
const available = (status: Status) => status === "supported" || status === "partial";
const errors: string[] = [];
for (const property of native) {
  const entry = reference.get(property.name);
  if (!entry) throw new Error(`Missing CSS reference page for ${property.name}`);
  if (property.status !== "supported" && property.status !== "partial") continue;
  if (!available(entry.status))
    errors.push(`${entry.name} is ${entry.status} in the reference and ${property.status} in sk-css`);
  for (const value of entry.values ?? []) {
    if (!available(value.status) && property.applied.includes(value.value)) {
      errors.push(`${entry.name}: ${value.value} is ${value.status} in the reference and accepted by sk-css`);
    }
  }
}
if (errors.length) throw new Error(errors.join("\n"));
