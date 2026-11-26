import { blend } from "./blend";
import { color } from "./color";
import { ease } from "./ease";
import { light } from "./light";
import { math } from "./math";
import { noise } from "./noise";
import { sample } from "./sample";
import { sdf } from "./sdf";
import { categories, type Category, type ShaderEntry } from "./shared";
import { sk } from "./sk";

export { categories };
export type { Category, ShaderEntry };

export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {
  sk: "The contract every effect is written against: the attributes the compiler reads into the CSS schema, `IEffect`, `EffectInput` and its samplers, `SkTexture`, and the helpers that have always shipped with it. `import sk;`",
  "sk.color":
    "Colours with their encoding in the type: `Color` holds linear-light sRGB with straight alpha, constructors are named after what you hold, and conversions to sRGB, HSL, HSV, HWB, XYZ, Lab, LCH, OKLab and OKLCH follow CSS Color 4. Manipulation, `color-mix()`-style mixing, contrast, ΔE and harmonies. `import sk.color;`",
  "sk.math":
    "The few helpers the other modules share: `SK_PI` and `SK_TAU`, a `floorMod` that wraps negatives upwards, a division that never yields infinity, and range remapping. `import sk.math;`",
  "sk.blend":
    "CSS `mix-blend-mode` and Porter-Duff compositing on premultiplied colours, W3C formulas exactly: sixteen blend modes as types for `blend<M>`, fifteen operators for `composite<Op>`, and `Color` forms in sRGB-encoded or linear light. `import sk.blend;`",
  "sk.sdf":
    "2D signed distance fields in pixels, the convention of `input.sd`: exact shapes from circles to stars, unions, smooth unions and cut-outs, mirroring and repetition, and antialiased fills, strokes, feathers, glows and normals. `import sk.sdf;`",
  "sk.ease":
    "Easing as CSS defines it, `cubic-bezier()`, `steps()` and `linear()`, the Penner set, a closed-form spring, and envelopes for effects timed by `input.time` and pointer times. `import sk.ease;`",
  "sk.noise":
    "Procedural noise with documented ranges: integer PCG hashes identical on every GPU, Perlin, simplex and value noise in 2D and 3D with analytic derivatives, Worley cells, fractals (`fbm`, turbulence, ridged, domain warp, curl) over any noise type, and dither patterns. `import sk.noise;`",
  "sk.sample":
    "Kernels over the engine's samplers: Gaussian, box, disc, directional and radial blurs, chromatic aberration and dispersion, displacement, bicubic resampling, sharpening and mosaics, each with the reach to declare in `[SkBackdropReach]`. `import sk.sample;`",
  "sk.light":
    "Light for flat UI: normals from distances, bevels, height fields and normal maps; Fresnel and Snell refraction as a pixel offset; diffuse, Blinn-Phong, GGX, rim and anisotropic highlights; shadows that match CSS `box-shadow`. `import sk.light;`",
};

export const entries: readonly ShaderEntry[] = [
  ...sk,
  ...color,
  ...math,
  ...blend,
  ...sdf,
  ...ease,
  ...noise,
  ...sample,
  ...light,
];
