//! The RGB spaces `<input type=color>` keeps its value in, and `color(display-p3-linear …)`, which
//! lightningcss does not parse: it is read here and, in declarations, rewritten to `xyz-d65`.

use lightningcss::traits::Parse as _;
use lightningcss::values::color::{ColorSpace as _, CssColor, PredefinedColor};

/// An RGB space a color can be read into with [`crate::parse_color_in`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RgbSpace {
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
}

impl RgbSpace {
    pub fn is_linear(self) -> bool {
        matches!(self, RgbSpace::SrgbLinear | RgbSpace::DisplayP3Linear)
    }

    fn is_p3(self) -> bool {
        matches!(self, RgbSpace::DisplayP3 | RgbSpace::DisplayP3Linear)
    }
}

const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_390_799_265_959_34,
        0.357_584_339_383_878,
        0.180_480_788_401_834_3,
    ],
    [
        0.212_639_005_871_510_27,
        0.715_168_678_767_756,
        0.072_192_315_360_733_71,
    ],
    [
        0.019_330_818_715_591_82,
        0.119_194_779_794_625_98,
        0.950_532_152_249_660_7,
    ],
];
const XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [
        3.240_969_941_904_522_6,
        -1.537_383_177_570_094,
        -0.498_610_760_293_003_4,
    ],
    [
        -0.969_243_636_280_879_6,
        1.875_967_501_507_720_2,
        0.041_555_057_407_175_59,
    ],
    [
        0.055_630_079_696_993_66,
        -0.203_976_958_888_976_52,
        1.056_971_514_242_878_6,
    ],
];
const P3_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.486_570_948_648_216_2,
        0.265_667_693_169_093_06,
        0.198_217_285_234_362_5,
    ],
    [
        0.228_974_564_069_748_8,
        0.691_738_521_836_506_4,
        0.079_286_914_093_745,
    ],
    [0.0, 0.045_113_381_858_902_64, 1.043_944_368_900_976],
];
const XYZ_TO_P3: [[f64; 3]; 3] = [
    [
        2.493_496_911_941_425,
        -0.931_383_617_919_123_9,
        -0.402_710_784_450_716_84,
    ],
    [
        -0.829_488_969_561_574_7,
        1.762_664_060_318_346_3,
        0.023_624_685_841_943_577,
    ],
    [
        0.035_845_830_243_784_47,
        -0.076_172_389_268_041_82,
        0.956_884_524_007_687_2,
    ],
];

fn mul(m: &[[f64; 3]; 3], [r, g, b]: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * r + row[1] * g + row[2] * b)
}

/// sRGB's transfer function, extended past 0 and 1 by symmetry as CSS does.
pub fn to_linear(c: f32) -> f32 {
    let a = c.abs();
    if a <= 0.040_45 {
        c / 12.92
    } else {
        c.signum() * ((a + 0.055) / 1.055).powf(2.4)
    }
}

pub fn from_linear(c: f32) -> f32 {
    let a = c.abs();
    if a <= 0.003_130_8 {
        c * 12.92
    } else {
        c.signum() * (1.055 * a.powf(1.0 / 2.4) - 0.055)
    }
}

/// Converts RGB components between the spaces, unclipped. Display P3 shares sRGB's transfer.
pub fn convert_rgb(rgb: [f32; 3], from: RgbSpace, to: RgbSpace) -> [f32; 3] {
    if from == to {
        return rgb;
    }
    let lin = if from.is_linear() {
        rgb
    } else {
        rgb.map(to_linear)
    };
    let lin = if from.is_p3() == to.is_p3() {
        lin
    } else {
        let (into_xyz, out_of_xyz) = if from.is_p3() {
            (&P3_TO_XYZ, &XYZ_TO_SRGB)
        } else {
            (&SRGB_TO_XYZ, &XYZ_TO_P3)
        };
        mul(out_of_xyz, mul(into_xyz, lin.map(f64::from))).map(|v| v as f32)
    };
    if to.is_linear() {
        lin
    } else {
        lin.map(from_linear)
    }
}

/// The components of `color(display-p3-linear …)`, `None` for any other text. `none` reads as 0.
pub(crate) fn display_p3_linear(text: &str) -> Option<[f32; 4]> {
    let rest = strip_ci(text.trim(), "color(")?.trim_start();
    let after = strip_ci(rest, "display-p3-linear")?;
    if !after.starts_with(|c: char| c.is_ascii_whitespace() || c == ')' || c == '/') {
        return None;
    }
    match CssColor::parse_string(&format!("color(srgb-linear{after}")).ok()? {
        CssColor::Predefined(p) => match *p {
            PredefinedColor::SRGBLinear(c) => {
                let c = c.resolve_missing();
                Some([c.r, c.g, c.b, c.alpha])
            }
            _ => None,
        },
        _ => None,
    }
}

fn strip_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// `text` with every `color(display-p3-linear …)` written as the same color in `xyz-d65`, `None`
/// when there is none.
pub(crate) fn rewrite(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("display-p3-linear") {
        return None;
    }
    let bytes = lower.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut from) = (0, 0);
    while let Some(at) = lower[from..].find("color(").map(|i| from + i) {
        from = at + "color(".len();
        let joined = at > 0
            && (bytes[at - 1].is_ascii_alphanumeric() || matches!(bytes[at - 1], b'-' | b'_'));
        if joined {
            continue;
        }
        let Some(end) = closing(bytes, from) else {
            break;
        };
        if let Some([r, g, b, a]) = display_p3_linear(&text[at..=end]) {
            let [x, y, z] = mul(&P3_TO_XYZ, [r, g, b].map(f64::from));
            out.push_str(&text[copied..at]);
            out.push_str(&format!("color(xyz-d65 {x} {y} {z} / {a})"));
            copied = end + 1;
        }
        from = end + 1;
    }
    if copied == 0 {
        return None;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

/// The `)` closing the parenthesis opened just before `from`.
fn closing(bytes: &[u8], from: usize) -> Option<usize> {
    let mut depth = 1;
    for (i, &b) in bytes.iter().enumerate().skip(from) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_color, parse_color_in};

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn display_p3_linear_parses_exactly_with_alpha_percentages_and_none() {
        assert_eq!(
            display_p3_linear("color(display-p3-linear 2 0.5 0 / 0.25)"),
            Some([2.0, 0.5, 0.0, 0.25])
        );
        assert_eq!(
            display_p3_linear(" COLOR( Display-P3-Linear 50% none 1) "),
            Some([0.5, 0.0, 1.0, 1.0])
        );
        assert_eq!(display_p3_linear("color(display-p3 1 0 0)"), None);
        assert_eq!(display_p3_linear("color(display-p3-linearx 1 0 0)"), None);
        assert_eq!(display_p3_linear("color(display-p3-linear 1 0)"), None);
    }

    #[test]
    fn colors_read_into_each_space_unclipped() {
        let red = parse_color_in("color(display-p3 1 0 0)", RgbSpace::Srgb).unwrap();
        assert!(
            red[0] > 1.0 && red[1] < 0.0,
            "P3 red is outside sRGB: {red:?}"
        );
        let back = parse_color_in("red", RgbSpace::DisplayP3).unwrap();
        assert!(
            close(&back, &[0.917_488, 0.200_287, 0.138_561, 1.0]),
            "{back:?}"
        );
        assert_eq!(
            parse_color_in("color(srgb-linear 4 2 0.5 / 0.5)", RgbSpace::SrgbLinear),
            Some([4.0, 2.0, 0.5, 0.5])
        );
        let lin = parse_color_in("#808080", RgbSpace::SrgbLinear).unwrap();
        assert!(
            close(&lin, &[0.215_861, 0.215_861, 0.215_861, 1.0]),
            "{lin:?}"
        );
        assert_eq!(
            parse_color_in("color(display-p3-linear 2 0 0)", RgbSpace::DisplayP3Linear),
            Some([2.0, 0.0, 0.0, 1.0])
        );
        let p3 = parse_color_in("color(display-p3-linear 1 1 1)", RgbSpace::Srgb).unwrap();
        assert!(close(&p3, &[1.0, 1.0, 1.0, 1.0]), "{p3:?}");
        assert_eq!(parse_color_in("currentcolor", RgbSpace::DisplayP3), None);
        assert_eq!(
            parse_color("color(srgb-linear 4 0 0)"),
            Some([1.0, 0.0, 0.0, 1.0]),
            "painting clips"
        );
    }

    #[test]
    fn conversions_round_trip_through_every_space() {
        let rgb = [0.9, 0.3, 0.05];
        for (a, b) in [
            (RgbSpace::Srgb, RgbSpace::DisplayP3),
            (RgbSpace::Srgb, RgbSpace::DisplayP3Linear),
            (RgbSpace::DisplayP3, RgbSpace::SrgbLinear),
            (RgbSpace::SrgbLinear, RgbSpace::Srgb),
        ] {
            let there = convert_rgb(rgb, a, b);
            assert!(close(&convert_rgb(there, b, a), &rgb), "{a:?} -> {b:?}");
        }
        assert!(close(
            &convert_rgb([2.0, 0.0, 0.0], RgbSpace::SrgbLinear, RgbSpace::Srgb),
            &[from_linear(2.0), 0.0, 0.0]
        ));
    }

    #[test]
    fn declarations_get_display_p3_linear_as_xyz() {
        let out =
            rewrite("background: color(display-p3-linear 1 0 0 / 0.5); color: color(srgb 1 0 0)")
                .unwrap();
        assert!(
            out.starts_with("background: color(xyz-d65 0.48657"),
            "{out}"
        );
        assert!(out.ends_with(" / 0.5); color: color(srgb 1 0 0)"), "{out}");
        let back = parse_color_in(
            &out["background: ".len()..out.find(';').unwrap()],
            RgbSpace::DisplayP3Linear,
        )
        .unwrap();
        assert!(close(&back, &[1.0, 0.0, 0.0, 0.5]), "{back:?}");
        assert_eq!(rewrite("color: color(display-p3 1 0 0)"), None);
        assert_eq!(rewrite("--x: color(display-p3-linear 1 0"), None);
    }
}
