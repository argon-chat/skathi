import { entries, type Status } from "../docs/src/data/css/entries";

interface NativeProperty {
  name: string;
  status: "supported" | "partial" | "ignored" | "unsupported" | "dropped";
  applied: string[];
}

const native: NativeProperty[] = await Bun.stdin.json();
const reference = new Map(
  entries
    .filter((entry) => entry.kind === "property" || entry.kind === "shorthand")
    .map((entry) => [entry.name, entry]),
);
const available = (status: Status) => status === "supported" || status === "partial";
const errors: string[] = [];
for (const property of native) {
  if (property.status !== "supported" && property.status !== "partial") continue;
  const entry = reference.get(property.name);
  if (!entry) continue;
  if (!available(entry.status))
    errors.push(`${entry.name} is ${entry.status} in the reference and ${property.status} in sk-css`);
  for (const value of entry.values ?? []) {
    if (!available(value.status) && property.applied.includes(value.value)) {
      errors.push(`${entry.name}: ${value.value} is ${value.status} in the reference and accepted by sk-css`);
    }
  }
}
if (errors.length) throw new Error(errors.join("\n"));
