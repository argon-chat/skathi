import manifest from "../../../../crates/sk-effects/generated/effects.json";
import { supported, type ShaderEntry } from "./shared";

export const builtinEffects: (ShaderEntry & { callSignature: string })[] = manifest.effects.map((effect) => ({
  ...supported,
  slug: `builtin-${effect.name}`,
  name: `${effect.name}()`,
  kind: "effect",
  category: "Built-in effects",
  summary: effect.doc ?? `Built-in effect for \`--shading\`.`,
  callSignature: effect.signature,
  syntax: `--shading: ${effect.signature};`,
  syntaxLanguage: "css",
  source: effect.source,
  params: effect.params.map((param) => ({
    name: param.name,
    type: param.kind,
    description: param.range
      ? `Clamped to ${param.range[0]}…${param.range[1]}.`
      : param.required
        ? "Required."
        : "Optional.",
    optional: !param.required,
    ...(param.required ? {} : { default: defaultValue(param) }),
  })),
}));

function defaultValue(param: (typeof manifest.effects)[number]["params"][number]): string {
  const [r, g, b, a] = param.default;
  if (param.kind === "color") return `color(srgb ${r} ${g} ${b} / ${a})`;
  if (param.kind === "texture") return `fallback RGBA (${param.default.join(", ")})`;
  return `${r}${param.kind === "length" ? "px" : ""}`;
}
