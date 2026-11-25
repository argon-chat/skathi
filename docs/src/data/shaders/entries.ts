import { color } from "./color";
import { categories, type Category, type ShaderEntry } from "./shared";
import { sk } from "./sk";

export { categories };
export type { Category, ShaderEntry };

export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {
  sk: "The contract every effect is written against: the attributes the compiler reads into the CSS schema, `IEffect`, `EffectInput` and its samplers, `SkTexture`, and the helpers that have always shipped with it. `import sk;`",
  "sk.color":
    "Colours with their encoding in the type: `Color` holds linear-light sRGB with straight alpha, constructors are named after what you hold, and conversions to sRGB, HSL, HSV, HWB, XYZ, Lab, LCH, OKLab and OKLCH follow CSS Color 4. Manipulation, `color-mix()`-style mixing, contrast, ΔE and harmonies. `import sk.color;`",
};

export const entries: readonly ShaderEntry[] = [...sk, ...color];
