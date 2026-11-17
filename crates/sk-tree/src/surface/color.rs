//! Half-float surfaces: linear extended sRGB (scRGB, 1.0 = SDR white) shown on the 8-bit sRGB
//! window. Each channel is clamped to [0, 1] (NaN to 0), then encoded with the exact sRGB curve
//! gpui's gradients use; alpha is clamped and scaled. The CPU table and the GPU shaders below agree
//! to within the GPU's float-to-unorm rounding.

use std::sync::OnceLock;

/// IEEE 754 binary16 bits to `f32`.
pub(crate) fn half_to_f32(bits: u16) -> f32 {
    let exponent = (bits >> 10) & 0x1f;
    let mantissa = u32::from(bits & 0x3ff);
    let magnitude = match exponent {
        0 => mantissa as f32 / 16_777_216.0,
        31 if mantissa == 0 => f32::INFINITY,
        31 => f32::NAN,
        _ => f32::from_bits(((u32::from(exponent) + 112) << 23) | (mantissa << 13)),
    };
    if bits & 0x8000 != 0 {
        -magnitude
    } else {
        magnitude
    }
}

fn unit(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// A linear channel as the 8-bit sRGB value the window shows.
pub(crate) fn encode(linear: f32) -> u8 {
    let c = unit(linear);
    let encoded = if c < 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

pub(crate) fn encode_alpha(alpha: f32) -> u8 {
    (unit(alpha) * 255.0).round() as u8
}

struct Tables {
    color: Box<[u8]>,
    alpha: Box<[u8]>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables {
        color: (0..=u16::MAX).map(|h| encode(half_to_f32(h))).collect(),
        alpha: (0..=u16::MAX)
            .map(|h| encode_alpha(half_to_f32(h)))
            .collect(),
    })
}

/// Maps half-float RGBA rows (`stride` values apart) to packed BGRA.
pub(crate) fn rgba16f_to_bgra8(pixels: &[u16], width: u32, height: u32, stride: usize) -> Vec<u8> {
    let t = tables();
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        for px in pixels[y * stride..y * stride + row].chunks_exact(4) {
            out.extend_from_slice(&[
                t.color[usize::from(px[2])],
                t.color[usize::from(px[1])],
                t.color[usize::from(px[0])],
                t.alpha[usize::from(px[3])],
            ]);
        }
    }
    out
}

/// The same mapping on the GPU: a fullscreen triangle that loads texel for pixel.
#[cfg(windows)]
pub(super) const HLSL: &str = r#"
Texture2D<float4> frame : register(t0);

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 at = float2((id << 1) & 2, id & 2);
    return float4(at * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 ps(float4 at : SV_Position) : SV_Target {
    float4 texel = frame.Load(int3(at.xy, 0));
    float3 c = saturate(texel.rgb);
    float3 srgb = lerp(c * 12.92, 1.055 * pow(c, 1.0 / 2.4) - 0.055, step(0.0031308, c));
    return float4(srgb, saturate(texel.a));
}
"#;

/// Compiles an entry point of [`HLSL`] once per process; device-independent bytecode.
#[cfg(windows)]
pub(super) fn bytecode(entry: Shader) -> Result<&'static [u8], String> {
    use windows::core::PCSTR;
    use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
    use windows::Win32::Graphics::Direct3D::ID3DBlob;

    static CODE: [OnceLock<Result<Vec<u8>, String>>; 4] = [
        const { OnceLock::new() },
        const { OnceLock::new() },
        const { OnceLock::new() },
        const { OnceLock::new() },
    ];
    let (index, name, target): (usize, &[u8], &[u8]) = match entry {
        Shader::Vertex4 => (0, b"vs\0", b"vs_4_0\0"),
        Shader::Pixel4 => (1, b"ps\0", b"ps_4_0\0"),
        Shader::Vertex5 => (2, b"vs\0", b"vs_5_0\0"),
        Shader::Pixel5 => (3, b"ps\0", b"ps_5_0\0"),
    };
    let blob_bytes = |blob: &ID3DBlob| {
        // SAFETY: a blob's pointer covers its size for its life.
        unsafe {
            std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
        }
        .to_vec()
    };
    CODE[index]
        .get_or_init(|| {
            let mut code: Option<ID3DBlob> = None;
            let mut errors: Option<ID3DBlob> = None;
            // SAFETY: the source, entry and target outlive the call; the names are NUL-terminated.
            let compiled = unsafe {
                D3DCompile(
                    HLSL.as_ptr().cast(),
                    HLSL.len(),
                    PCSTR::from_raw(b"surface_rgba16f\0".as_ptr()),
                    None,
                    None,
                    PCSTR::from_raw(name.as_ptr()),
                    PCSTR::from_raw(target.as_ptr()),
                    D3DCOMPILE_OPTIMIZATION_LEVEL3,
                    0,
                    &mut code,
                    Some(&mut errors),
                )
            };
            match (compiled, code) {
                (Ok(()), Some(code)) => Ok(blob_bytes(&code)),
                (result, _) => Err(format!(
                    "compiling the half-float surface shader: {result:?} {}",
                    errors
                        .map(|e| String::from_utf8_lossy(&blob_bytes(&e)).into_owned())
                        .unwrap_or_default()
                )),
            }
        })
        .as_deref()
        .map_err(Clone::clone)
}

#[cfg(windows)]
#[derive(Clone, Copy)]
pub(super) enum Shader {
    /// Direct3D 11, which may run at feature level 10.
    Vertex4,
    Pixel4,
    /// Direct3D 12.
    Vertex5,
    Pixel5,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `f32` to binary16, rounding to nearest; normals only, as the tests use.
    pub(crate) fn f32_to_half(value: f32) -> u16 {
        let bits = value.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        if value.is_nan() {
            return sign | 0x7e00;
        }
        let exponent = ((bits >> 23) & 0xff) as i32 - 112;
        if value == 0.0 || exponent <= 0 {
            sign
        } else if exponent >= 31 {
            sign | 0x7c00
        } else {
            let truncated = sign | ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16;
            truncated + ((bits >> 12) & 1) as u16
        }
    }

    /// The sRGB EOTF: what a producer that linearises 8-bit sRGB itself computes.
    pub(crate) fn eotf(code: u8) -> f32 {
        let c = f64::from(code) / 255.0;
        (if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }) as f32
    }

    /// Every sRGB code linearised to halves: 256 RGBA pixels, colour and alpha both the code.
    pub(crate) fn ramp() -> Vec<u16> {
        (0..=255u8)
            .flat_map(|code| {
                let (c, a) = (
                    f32_to_half(eotf(code)),
                    f32_to_half(f32::from(code) / 255.0),
                );
                [c, c, c, a]
            })
            .collect()
    }

    /// Linear values and the bytes an sRGB display shows for them; above 1.0 and below 0 clamp.
    pub(crate) const KNOWN: [(f32, u8); 9] = [
        (0.0, 0),
        (0.003_130_8, 10),
        (0.05, 63),
        (0.2159, 128),
        (0.5, 188),
        (1.0, 255),
        (4.0, 255),
        (-0.5, 0),
        (f32::INFINITY, 255),
    ];

    #[test]
    fn halves_decode_normals_subnormals_and_specials() {
        for v in [0.0, 1.0, -2.5, 0.5, 65504.0, 0.000_061_035_156] {
            assert_eq!(half_to_f32(f32_to_half(v)), v);
        }
        assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(half_to_f32(0x7c00), f32::INFINITY);
        assert!(half_to_f32(0x7e00).is_nan());
    }

    #[test]
    fn linear_light_clamps_then_takes_the_exact_srgb_curve() {
        for (linear, byte) in KNOWN {
            assert_eq!(encode(linear), byte, "{linear}");
        }
        assert_eq!(encode(f32::NAN), 0);
        assert_eq!(encode_alpha(0.5), 128);
        assert_eq!(encode_alpha(2.0), 255);
    }

    #[test]
    fn every_srgb_code_linearised_to_a_half_comes_back_unchanged() {
        let bgra = rgba16f_to_bgra8(&ramp(), 256, 1, 1024);
        for (code, px) in bgra.chunks_exact(4).enumerate() {
            assert_eq!(px, [code as u8; 4], "code {code}");
        }
    }

    #[test]
    fn rows_map_to_bgra_through_the_table() {
        let h = f32_to_half;
        let pixels = [
            h(1.0),
            h(0.5),
            h(0.0),
            h(1.0),
            h(8.0),
            h(0.2159),
            h(-1.0),
            h(0.5),
            9,
            9,
            9,
            9,
        ];
        assert_eq!(
            rgba16f_to_bgra8(&pixels, 2, 1, 12),
            [0, 188, 255, 255, 0, 128, 255, 128]
        );
    }
}
