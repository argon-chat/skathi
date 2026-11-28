---
title: Shader library
label: Shader library
description: "Public Slang modules for writing Skathi shaders and the signatures of built-in CSS effects."
section: Reference
order: 6.5
---

# Shader library

An app's `--shading` effects are written in [Slang](https://shader-slang.org/) under `shaders/`.
They import Skathi's shader modules by name; `sk build`, `sk dev` and `sk check` provide them, so
there is nothing to install or copy into the app.

```slang
import sk;       // EffectInput and IEffect
import sk.color; // Color and colour-space conversions
```

The [shader library guide](/guides/shader-library/) walks through writing an effect with them.

| Module                                    | What it gives                                                                                                 |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| [`sk`](#sk)                               | Effect attributes, `IEffect`, pixel coordinates and backdrop sampling through `EffectInput`, and `SkTexture`. |
| [`sk.color`](#skcolor)                    | Colour-space conversions, mixing, contrast, colour differences and harmonies.                                 |
| [`sk.math`](#skmath)                      | `SK_PI`, `SK_TAU`, `floorMod`, `safeDiv`, `safeAtan2` and `remap`.                                            |
| [`sk.blend`](#skblend)                    | CSS blend modes with `blend<M>` and Porter-Duff compositing with `composite<Op>`.                             |
| [`sk.sdf`](#sksdf)                        | Distance fields for shapes, smooth unions, repetition, strokes and glows.                                     |
| [`sk.ease`](#skease)                      | CSS easing curves, stepped timing, springs and time envelopes.                                                |
| [`sk.noise`](#sknoise)                    | Hashes, Perlin, simplex and Worley noise, fractals, curl and dithering.                                       |
| [`sk.sample`](#sksample)                  | Blur kernels, dispersion, displacement, bicubic sampling and sharpening.                                      |
| [`sk.light`](#sklight)                    | Normals, bevels, refraction, highlights and shadows.                                                          |
| [`sk.tone`](#sktone)                      | Tone curves, exposure, soft clipping, nits and PQ/HLG encoding.                                               |
| [Built-in CSS effects](#built-in-effects) | Effect arguments, ranges and defaults for `--shading`.                                                        |

## Conventions

- **Colour encodings.** An `[SkColor]` parameter is straight-alpha, sRGB-encoded RGBA. Samples of
  the backdrop, the content and colour images are premultiplied and sRGB-encoded. `shade` returns
  premultiplied, sRGB-encoded RGBA unless the effect opts into straight alpha. `Color` names each of
  these in its constructors, so a conversion is never implicit. A `[SkBackdropLinear]` effect reads
  its backdrop and returns its result in premultiplied linear light instead, 1.0 at SDR white.
- **Units.** Lengths and `input.px` are device pixels; `input.local` runs 0..1 across the element.
  Hues are degrees.
- **Cost.** Modules are compiled into each effect that imports them, and only what the effect calls
  reaches its shader. A module costs a little compile time when imported, nothing at run time when
  unused.
- **Backends.** Every function gives the same result on DirectX 12, Metal and wgpu (Vulkan and
  DirectX 12), which the test suite checks case by case on each of them.
