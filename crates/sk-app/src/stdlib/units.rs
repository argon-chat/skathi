//! `skathi:units`: `DataSize`, `Frequency` and `Angle`.

use std::ffi::c_void;
use std::sync::OnceLock;

use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};
use super::time::{ms_arg, span_new, trimmed};

fn host_class<T>(cell: &OnceLock<usize>, name: &str) -> JSClassRef {
    *cell.get_or_init(|| unsafe { make_host_class::<T>(name) } as usize) as JSClassRef
}

/// The unit as written, its size in bytes, and the name `Intl.NumberFormat` knows it by.
const SIZES: &[(&str, f64, &str)] = &[
    ("B", 1.0, "byte"),
    ("kB", 1e3, "kilobyte"),
    ("MB", 1e6, "megabyte"),
    ("GB", 1e9, "gigabyte"),
    ("TB", 1e12, "terabyte"),
    ("PB", 1e15, "petabyte"),
    ("KiB", 1024.0, ""),
    ("MiB", 1_048_576.0, ""),
    ("GiB", 1_073_741_824.0, ""),
    ("TiB", 1_099_511_627_776.0, ""),
    ("PiB", 1_125_899_906_842_624.0, ""),
];
const DECIMAL: std::ops::Range<usize> = 0..6;
const BINARY: [usize; 6] = [0, 6, 7, 8, 9, 10];

fn parse_size(text: &str) -> Result<f64, String> {
    let text = text.trim();
    let digits = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    let value: f64 = text[..digits]
        .parse()
        .map_err(|_| "it does not start with a number".to_owned())?;
    let unit = text[digits..].trim();
    if unit.is_empty() {
        return Ok(value);
    }
    match SIZES
        .iter()
        .find(|(name, ..)| name.eq_ignore_ascii_case(unit))
    {
        Some((_, scale, _)) => Ok(value * scale),
        None => Err(format!(
            "\"{unit}\" is not a unit; use B, kB, MB, GB, TB, PB or KiB, MiB, GiB, TiB, PiB"
        )),
    }
}

/// The largest unit of the family that leaves at least 1, as an index into `SIZES`.
fn fitting(bytes: f64, binary: bool) -> usize {
    let family: Vec<usize> = if binary {
        BINARY.to_vec()
    } else {
        DECIMAL.collect()
    };
    family
        .iter()
        .rev()
        .copied()
        .find(|i| bytes.abs() >= SIZES[*i].1)
        .unwrap_or(0)
}

struct DataSize(f64);
struct Frequency(f64);
struct Angle(f64);

static DATA_SIZE: OnceLock<usize> = OnceLock::new();
static FREQUENCY: OnceLock<usize> = OnceLock::new();
static ANGLE: OnceLock<usize> = OnceLock::new();

unsafe fn finite(g: *mut c_void, v: u64, what: &str) -> Js<f64> {
    let n = number(g, v)?;
    if !n.is_finite() {
        return range_error(g, &format!("{what} must be a finite number"));
    }
    Ok(n)
}

unsafe fn size_new(g: *mut c_void, bytes: f64) -> u64 {
    let class = host_class::<DataSize>(&DATA_SIZE, "DataSize");
    host_create(g, class, prototype(g, "DataSize"), DataSize(bytes))
}

unsafe fn this_size(g: *mut c_void, this: u64) -> Js<f64> {
    match host_data::<DataSize>(g, host_class::<DataSize>(&DATA_SIZE, "DataSize"), this) {
        Some(size) => Ok(size.0),
        None => type_error(g, "this is not a DataSize"),
    }
}

std_fn!(fn size_bytes_of(g, args) {
    Ok(size_new(g, finite(g, args.arg(0).0, "DataSize.bytes: the count")?))
});

std_fn!(fn size_parse(g, args) {
    let text = string(g, args.arg(0).0, "DataSize.parse: the text")?;
    match parse_size(&text) {
        Ok(bytes) => Ok(size_new(g, bytes)),
        Err(reason) => syntax_error(g, &format!("DataSize.parse: \"{text}\" is not a size: {reason}")),
    }
});

std_fn!(fn size_bytes(g, args) { Ok(enc_f64(this_size(g, args.this().0)?)) });

std_fn!(fn size_to(g, args) {
    let bytes = this_size(g, args.this().0)?;
    let names: Vec<&str> = SIZES.iter().map(|(name, ..)| *name).collect();
    let unit = choice(g, args.arg(0).0, &names, usize::MAX, "DataSize.to: the unit")?;
    match SIZES.get(unit) {
        Some((_, scale, _)) => Ok(enc_f64(bytes / scale)),
        None => type_error(g, "DataSize.to: the unit is missing"),
    }
});

std_fn!(fn size_format(g, args) {
    let bytes = this_size(g, args.this().0)?;
    let options = args.arg(0).0;
    let binary = to_bool(g, option(g, options, "binary"));
    let locale = option(g, options, "locale");
    let (name, scale, intl) = SIZES[fitting(bytes, binary)];
    let settings = new_object(g, UNDEF);
    define(g, settings, "maximumFractionDigits", enc_i32(1), 0);
    if !intl.is_empty() {
        define(g, settings, "style", js_string(g, "unit"), 0);
        define(g, settings, "unit", js_string(g, intl), 0);
        define(g, settings, "unitDisplay", js_string(g, "short"), 0);
    }
    let class = get(g, get(g, glue::rjs_global(g), "Intl"), "NumberFormat");
    let formatter = construct(g, class, &[locale, settings])?;
    let text = call(g, get(g, formatter, "format"), formatter, &[enc_f64(bytes / scale)])
        .map_err(|_| Thrown)?;
    if !intl.is_empty() {
        return Ok(text);
    }
    let number = read_string(g, text).unwrap_or_default();
    Ok(js_string(g, &format!("{number} {name}")))
});

unsafe fn frequency_new(g: *mut c_void, hz: f64) -> u64 {
    let class = host_class::<Frequency>(&FREQUENCY, "Frequency");
    host_create(g, class, prototype(g, "Frequency"), Frequency(hz))
}

unsafe fn this_frequency(g: *mut c_void, this: u64) -> Js<f64> {
    match host_data::<Frequency>(g, host_class::<Frequency>(&FREQUENCY, "Frequency"), this) {
        Some(frequency) => Ok(frequency.0),
        None => type_error(g, "this is not a Frequency"),
    }
}

std_fn!(fn frequency_hz_of(g, args) {
    let hz = finite(g, args.arg(0).0, "Frequency.hz: the value")?;
    if hz < 0.0 {
        return range_error(g, "Frequency.hz: the value must not be negative");
    }
    Ok(frequency_new(g, hz))
});

std_fn!(fn frequency_from_period(g, args) {
    let ms = ms_arg(g, args.arg(0).0, "Frequency.fromPeriod: the period")?;
    if ms <= 0.0 {
        return range_error(g, "Frequency.fromPeriod: the period must be longer than zero");
    }
    Ok(frequency_new(g, 1_000.0 / ms))
});

std_fn!(fn frequency_hz(g, args) { Ok(enc_f64(this_frequency(g, args.this().0)?)) });

std_fn!(fn frequency_period(g, args) {
    let hz = this_frequency(g, args.this().0)?;
    if hz == 0.0 {
        return range_error(g, "Frequency.period: 0 Hz has no period");
    }
    Ok(span_new(g, 1_000.0 / hz))
});

fn frequency_text(hz: f64) -> String {
    let (scale, unit) = [(1e9, "GHz"), (1e6, "MHz"), (1e3, "kHz")]
        .into_iter()
        .find(|(scale, _)| hz >= *scale)
        .unwrap_or((1.0, "Hz"));
    format!("{} {unit}", trimmed(hz / scale, 3))
}

std_fn!(fn frequency_to_string(g, args) {
    Ok(js_string(g, &frequency_text(this_frequency(g, args.this().0)?)))
});

unsafe fn angle_new(g: *mut c_void, degrees: f64) -> u64 {
    let class = host_class::<Angle>(&ANGLE, "Angle");
    host_create(g, class, prototype(g, "Angle"), Angle(degrees))
}

unsafe fn this_angle(g: *mut c_void, this: u64) -> Js<f64> {
    match host_data::<Angle>(g, host_class::<Angle>(&ANGLE, "Angle"), this) {
        Some(angle) => Ok(angle.0),
        None => type_error(g, "this is not an Angle"),
    }
}

std_fn!(fn angle_deg(g, args) { Ok(angle_new(g, finite(g, args.arg(0).0, "Angle.deg: the value")?)) });

std_fn!(fn angle_rad(g, args) {
    Ok(angle_new(g, finite(g, args.arg(0).0, "Angle.rad: the value")?.to_degrees()))
});

std_fn!(fn angle_degrees(g, args) { Ok(enc_f64(this_angle(g, args.this().0)?)) });
std_fn!(fn angle_radians(g, args) { Ok(enc_f64(this_angle(g, args.this().0)?.to_radians())) });
std_fn!(fn angle_normalize(g, args) { Ok(angle_new(g, this_angle(g, args.this().0)?.rem_euclid(360.0))) });

std_fn!(fn angle_to_string(g, args) {
    Ok(js_string(g, &format!("{}°", trimmed(this_angle(g, args.this().0)?, 3))))
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let size = support::class(
        g,
        &ClassSpec {
            name: "DataSize",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("to", 1, size_to),
                ("format", 0, size_format),
                ("toString", 0, size_format),
                ("valueOf", 0, size_bytes),
            ],
            getters: &[("bytes", size_bytes)],
            accessors: &[],
        },
    );
    static_fn(g, size.ctor, "bytes", 1, size_bytes_of);
    static_fn(g, size.ctor, "parse", 1, size_parse);

    let frequency = support::class(
        g,
        &ClassSpec {
            name: "Frequency",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("toString", 0, frequency_to_string),
                ("valueOf", 0, frequency_hz),
            ],
            getters: &[("hz", frequency_hz), ("period", frequency_period)],
            accessors: &[],
        },
    );
    static_fn(g, frequency.ctor, "hz", 1, frequency_hz_of);
    static_fn(g, frequency.ctor, "fromPeriod", 1, frequency_from_period);

    let angle = support::class(
        g,
        &ClassSpec {
            name: "Angle",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("normalize", 0, angle_normalize),
                ("toString", 0, angle_to_string),
            ],
            getters: &[("degrees", angle_degrees), ("radians", angle_radians)],
            accessors: &[],
        },
    );
    static_fn(g, angle.ctor, "deg", 1, angle_deg);
    static_fn(g, angle.ctor, "rad", 1, angle_rad);

    let module = new_object(g, UNDEF);
    define(g, module, "DataSize", size.ctor, 0);
    define(g, module, "Frequency", frequency.ctor, 0);
    define(g, module, "Angle", angle.ctor, 0);
    define(g, std, "units", module, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_decimal_and_binary_units() {
        assert_eq!(parse_size("512"), Ok(512.0));
        assert_eq!(parse_size("1.5 GiB"), Ok(1.5 * 1_073_741_824.0));
        assert_eq!(parse_size("10mb"), Ok(1e7));
        assert!(parse_size("ten MB").is_err());
        assert!(parse_size("3 parsecs").is_err());
    }

    #[test]
    fn a_size_picks_the_unit_that_leaves_at_least_one() {
        assert_eq!(SIZES[fitting(999.0, false)].0, "B");
        assert_eq!(SIZES[fitting(1_000.0, false)].0, "kB");
        assert_eq!(SIZES[fitting(1_000.0, true)].0, "B");
        assert_eq!(SIZES[fitting(5e9, true)].0, "GiB");
        assert_eq!(SIZES[fitting(0.0, false)].0, "B");
    }

    #[test]
    fn a_frequency_prints_in_the_unit_it_reaches() {
        assert_eq!(frequency_text(60.0), "60 Hz");
        assert_eq!(frequency_text(44_100.0), "44.1 kHz");
        assert_eq!(frequency_text(2.4e9), "2.4 GHz");
    }
}
