//! `dynamic-range-limit` and `dynamic-range-limit-mix()`: how bright HDR content may be, as a
//! weighting of the three keywords.

/// The computed `dynamic-range-limit`: the weights of `standard`, `constrained` and `no-limit`,
/// summing to 1. The initial value is `no-limit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynamicRangeLimit {
    pub standard: f32,
    pub constrained: f32,
    pub no_limit: f32,
}

impl Default for DynamicRangeLimit {
    fn default() -> Self {
        Self::NO_LIMIT
    }
}

/// The headroom `constrained` allows: one stop above SDR white.
pub const CONSTRAINED_HEADROOM: f32 = 2.0;

impl DynamicRangeLimit {
    pub const STANDARD: Self = Self::weights(1., 0., 0.);
    pub const CONSTRAINED: Self = Self::weights(0., 1., 0.);
    pub const NO_LIMIT: Self = Self::weights(0., 0., 1.);

    const fn weights(standard: f32, constrained: f32, no_limit: f32) -> Self {
        Self {
            standard,
            constrained,
            no_limit,
        }
    }

    /// The value at `t` of a transition from `self` to `to`: the mix of the two at `1 - t` and `t`.
    pub fn interpolate(self, to: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self::weights(
            mix(self.standard, to.standard),
            mix(self.constrained, to.constrained),
            mix(self.no_limit, to.no_limit),
        )
        .normalized()
        .unwrap_or(to)
    }

    fn normalized(self) -> Option<Self> {
        let (s, c, n) = (
            self.standard.max(0.),
            self.constrained.max(0.),
            self.no_limit.max(0.),
        );
        let sum = s + c + n;
        (sum > 0. && sum.is_finite()).then(|| Self::weights(s / sum, c / sum, n / sum))
    }

    /// The brightest value content may reach, in units of SDR white, on a display that shows up to
    /// `display_headroom`: the keywords' limits mixed in stops (log2), as luminance is perceived.
    pub fn headroom(self, display_headroom: f32) -> f32 {
        let display = display_headroom.max(1.);
        let stops = self.constrained * CONSTRAINED_HEADROOM.min(display).log2()
            + self.no_limit * display.log2();
        stops.exp2()
    }

    /// The computed value as CSS: a keyword, or a mix of the keywords with their weights.
    pub fn to_css(self) -> String {
        let parts: Vec<(&str, f32)> = [
            ("standard", self.standard),
            ("constrained", self.constrained),
            ("no-limit", self.no_limit),
        ]
        .into_iter()
        .filter(|(_, w)| *w > 1e-6)
        .collect();
        match parts.as_slice() {
            [(keyword, _)] => (*keyword).to_string(),
            _ => format!(
                "dynamic-range-limit-mix({})",
                parts
                    .iter()
                    .map(|(k, w)| format!("{k} {}%", (w * 1e4).round() / 100.))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// `standard`, `constrained`, `no-limit` or `dynamic-range-limit-mix(<limit> <percentage>, …)`,
    /// with mixes nested in mixes.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        match lower.as_str() {
            "standard" => return Ok(Self::STANDARD),
            "constrained" => return Ok(Self::CONSTRAINED),
            "no-limit" => return Ok(Self::NO_LIMIT),
            _ => {}
        }
        let inner = lower
            .strip_prefix("dynamic-range-limit-mix(")
            .and_then(|rest| rest.strip_suffix(')'))
            .ok_or_else(|| {
                format!(
                    "`{text}` is not `standard`, `constrained`, `no-limit` or \
                     `dynamic-range-limit-mix()`"
                )
            })?;
        let items = split_commas(inner);
        if items.len() < 2 {
            return Err("`dynamic-range-limit-mix()` takes at least two limits".into());
        }
        let mut sum = Self::weights(0., 0., 0.);
        let mut total = 0.;
        for item in items {
            let (limit, percent) = split_percentage(item.trim())
                .ok_or_else(|| format!("`{}` needs one percentage", item.trim()))?;
            if !(0. ..=100.).contains(&percent) {
                return Err(format!("`{percent}%` is outside 0% to 100%"));
            }
            let limit = Self::parse(limit)?;
            let w = percent / 100.;
            sum.standard += limit.standard * w;
            sum.constrained += limit.constrained * w;
            sum.no_limit += limit.no_limit * w;
            total += w;
        }
        if total <= 0. {
            return Err("the percentages of `dynamic-range-limit-mix()` add up to 0%".into());
        }
        sum.normalized()
            .ok_or_else(|| "the percentages of `dynamic-range-limit-mix()` add up to 0%".into())
    }
}

fn split_commas(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// `<limit> <percentage>` in either order.
fn split_percentage(item: &str) -> Option<(&str, f32)> {
    let percent = |word: &str| word.strip_suffix('%')?.parse::<f32>().ok();
    if let Some((first, rest)) = item.split_once(char::is_whitespace) {
        if let Some(p) = percent(first) {
            return Some((rest.trim(), p));
        }
    }
    let (rest, last) = item.rsplit_once(char::is_whitespace)?;
    Some((rest.trim(), percent(last)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DynamicRangeLimit, b: DynamicRangeLimit) -> bool {
        (a.standard - b.standard).abs() < 1e-5
            && (a.constrained - b.constrained).abs() < 1e-5
            && (a.no_limit - b.no_limit).abs() < 1e-5
    }

    #[test]
    fn keywords_and_mixes_compute_to_weights() {
        assert_eq!(
            DynamicRangeLimit::parse("standard"),
            Ok(DynamicRangeLimit::STANDARD)
        );
        assert_eq!(
            DynamicRangeLimit::parse(" No-Limit "),
            Ok(DynamicRangeLimit::NO_LIMIT)
        );
        let mixed = DynamicRangeLimit::parse("dynamic-range-limit-mix(standard 30%, no-limit 70%)")
            .unwrap();
        assert!(
            close(mixed, DynamicRangeLimit::weights(0.3, 0., 0.7)),
            "{mixed:?}"
        );
        let reordered =
            DynamicRangeLimit::parse("dynamic-range-limit-mix(10% constrained, 30% no-limit)")
                .unwrap();
        assert!(
            close(reordered, DynamicRangeLimit::weights(0., 0.25, 0.75)),
            "{reordered:?}"
        );
    }

    #[test]
    fn mixes_nest() {
        let nested = DynamicRangeLimit::parse(
            "dynamic-range-limit-mix(dynamic-range-limit-mix(standard 50%, constrained 50%) 50%, \
             no-limit 50%)",
        )
        .unwrap();
        assert!(
            close(nested, DynamicRangeLimit::weights(0.25, 0.25, 0.5)),
            "{nested:?}"
        );
    }

    #[test]
    fn invalid_mixes_are_refused() {
        for bad in [
            "dynamic-range-limit-mix(standard 50%)",
            "dynamic-range-limit-mix(standard 0%, no-limit 0%)",
            "dynamic-range-limit-mix(standard 120%, no-limit 10%)",
            "dynamic-range-limit-mix(standard, no-limit 10%)",
            "high",
        ] {
            assert!(DynamicRangeLimit::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn interpolation_is_a_mix_of_the_endpoints() {
        let from = DynamicRangeLimit::STANDARD;
        let to = DynamicRangeLimit::NO_LIMIT;
        assert!(close(from.interpolate(to, 0.), from));
        assert!(close(from.interpolate(to, 1.), to));
        let mid = from.interpolate(to, 0.25);
        assert!(
            close(mid, DynamicRangeLimit::weights(0.75, 0., 0.25)),
            "{mid:?}"
        );
        let mixed = DynamicRangeLimit::weights(0.5, 0.5, 0.);
        assert!(close(
            mixed.interpolate(DynamicRangeLimit::CONSTRAINED, 0.5),
            DynamicRangeLimit::weights(0.25, 0.75, 0.)
        ));
    }

    #[test]
    fn the_headroom_mixes_in_stops() {
        assert_eq!(DynamicRangeLimit::STANDARD.headroom(8.), 1.);
        assert_eq!(DynamicRangeLimit::CONSTRAINED.headroom(8.), 2.);
        assert_eq!(DynamicRangeLimit::CONSTRAINED.headroom(1.5), 1.5);
        assert_eq!(DynamicRangeLimit::NO_LIMIT.headroom(8.), 8.);
        let half = DynamicRangeLimit::weights(0.5, 0., 0.5);
        assert!((half.headroom(4.) - 2.).abs() < 1e-5);
        assert_eq!(DynamicRangeLimit::NO_LIMIT.headroom(1.), 1.);
    }

    #[test]
    fn the_computed_value_serializes() {
        assert_eq!(DynamicRangeLimit::CONSTRAINED.to_css(), "constrained");
        assert_eq!(
            DynamicRangeLimit::weights(0.3, 0., 0.7).to_css(),
            "dynamic-range-limit-mix(standard 30%, no-limit 70%)"
        );
    }
}
