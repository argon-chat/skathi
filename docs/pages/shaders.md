---
title: Shader library
label: Shader library
description: "The Slang modules an effect imports: sk, the effect contract with its attributes, EffectInput and helpers, and sk.color, colours with their encoding in the type, conversions between CSS colour spaces, mixing, contrast and harmonies."
section: Reference
order: 6.5
---

# Shader library

An app's `--shading` effects are written in [Slang](https://shader-slang.org/) under `shaders/`.
They import Skathi's shader modules by name; `sk build`, `sk dev` and `sk check` provide them, so
there is nothing to install or copy into the app.

```slang
import sk;        // the effect contract: attributes, IEffect, EffectInput
import sk.color;  // Color, colour spaces, mixing, contrast
```

| Module                 | What it gives                                                                                   |
| ---------------------- | ----------------------------------------------------------------------------------------------- |
| [`sk`](#sk)            | `[SkEffect]` and the other attributes, `IEffect`, `EffectInput`, `SkTexture`, the `sk_` helpers |
| [`sk.color`](#skcolor) | `Color`, sRGB to OKLCH conversions, `mix`, lighten and darken, `contrast`, ΔE, harmonies        |

The [shader library guide](/guides/shader-library/) walks through writing an effect with them.

## Conventions

- **Colour encodings.** An `[SkColor]` parameter is straight-alpha, sRGB-encoded RGBA. Samples of
  the backdrop, the content and colour images are premultiplied and sRGB-encoded. `shade` returns
  premultiplied, sRGB-encoded RGBA unless the effect opts into straight alpha. `Color` names each of
  these in its constructors, so a conversion is never implicit.
- **Units.** Lengths and `input.px` are device pixels; `input.local` runs 0..1 across the element.
  Hues are degrees.
- **Cost.** Modules are compiled into each effect that imports them, and only what the effect calls
  reaches its shader. A module costs a little compile time when imported, nothing at run time when
  unused.
- **Backends.** Every function gives the same result on DirectX 12, Metal and wgpu (Vulkan and
  DirectX 12), which the test suite checks case by case on each of them.
