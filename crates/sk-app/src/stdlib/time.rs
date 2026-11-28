//! `skathi:time`: `TimeSpan`, `Stopwatch`, `Backoff` and `Deadline`. The classes that are all
//! promises and timers (`RateLimiter`, `RetryPolicy`, `Timer`) are in the runtime's `std/time.ts`.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};

const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 86_400_000.0;

fn host_class<T>(cell: &OnceLock<usize>, name: &str) -> JSClassRef {
    *cell.get_or_init(|| unsafe { make_host_class::<T>(name) } as usize) as JSClassRef
}

/// `n` with up to `decimals` digits after the point and no trailing zeros.
pub(super) fn trimmed(n: f64, decimals: usize) -> String {
    let text = format!("{n:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

struct Span(f64);

static SPAN: OnceLock<usize> = OnceLock::new();

pub(super) unsafe fn span_new(g: *mut c_void, ms: f64) -> u64 {
    unsafe {
        let class = host_class::<Span>(&SPAN, "TimeSpan");
        host_create(g, class, prototype(g, "TimeSpan"), Span(ms))
    }
}

pub(super) unsafe fn span_ms(g: *mut c_void, v: u64) -> Option<f64> {
    unsafe { host_data::<Span>(g, host_class::<Span>(&SPAN, "TimeSpan"), v).map(|span| span.0) }
}

/// Milliseconds given as a number or as a `TimeSpan`.
pub(super) unsafe fn ms_arg(g: *mut c_void, v: u64, what: &str) -> Js<f64> {
    unsafe {
        let ms = match span_ms(g, v) {
            Some(ms) => ms,
            None => number(g, v)?,
        };
        if !ms.is_finite() {
            return range_error(
                g,
                &format!("{what} must be a finite number of milliseconds or a TimeSpan"),
            );
        }
        Ok(ms)
    }
}

unsafe fn this_span(g: *mut c_void, this: u64) -> Js<f64> {
    unsafe {
        match span_ms(g, this) {
            Some(ms) => Ok(ms),
            None => type_error(g, "this is not a TimeSpan"),
        }
    }
}

/// `1h30m`, `250ms`, `1.5s`, or an ISO 8601 duration of weeks, days and time (`P1DT2H`).
fn parse_span(text: &str) -> Result<f64, String> {
    let trimmed = text.trim();
    let (negative, body) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let iso = body.starts_with(['P', 'p']);
    let mut rest = if iso { &body[1..] } else { body };
    let mut in_time = !iso;
    let mut total = 0.0;
    let mut parts = 0;
    loop {
        rest = rest.trim_start();
        if iso && rest.starts_with(['T', 't']) {
            in_time = true;
            rest = &rest[1..];
            continue;
        }
        if rest.is_empty() {
            break;
        }
        let digits = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        let value: f64 = rest[..digits]
            .parse()
            .map_err(|_| format!("a number was expected at \"{rest}\""))?;
        rest = rest[digits..].trim_start();
        let letters = rest
            .find(|c: char| !c.is_ascii_alphabetic() || (iso && matches!(c, 'T' | 't')))
            .unwrap_or(rest.len());
        let unit = rest[..letters].to_ascii_lowercase();
        let unit_len = if iso { unit.len().min(1) } else { unit.len() };
        let scale = match (&unit[..unit_len], iso, in_time) {
            ("w", true, false) => 7.0 * DAY,
            ("d", true, false) | ("d", false, _) => DAY,
            ("h", _, true) => HOUR,
            ("m", _, true) => MINUTE,
            ("s", _, true) => SECOND,
            ("ms", false, _) => 1.0,
            ("us", false, _) => 0.001,
            ("y", true, _) | ("m", true, false) => {
                return Err("years and months have no fixed length".into());
            }
            ("", ..) => return Err(format!("{value} has no unit; use d, h, m, s or ms")),
            (other, ..) => return Err(format!("\"{other}\" is not a unit; use d, h, m, s or ms")),
        };
        total += value * scale;
        parts += 1;
        rest = &rest[unit_len..];
    }
    if parts == 0 {
        return Err("it is empty".into());
    }
    Ok(if negative { -total } else { total })
}

fn span_text(ms: f64) -> String {
    let mut left = ms.abs();
    let mut out = String::from(if ms < 0.0 { "-" } else { "" });
    if left < SECOND {
        if left == 0.0 {
            return "0s".into();
        }
        return format!("{out}{}ms", trimmed(left, 3));
    }
    for (scale, unit) in [(DAY, "d"), (HOUR, "h"), (MINUTE, "m")] {
        let whole = (left / scale).floor();
        if whole > 0.0 {
            out.push_str(&format!("{whole}{unit}"));
            left -= whole * scale;
        }
    }
    if left > 0.0 {
        out.push_str(&format!("{}s", trimmed(left / SECOND, 6)));
    }
    out
}

/// Days, hours, minutes, seconds and whole milliseconds, each with the sign of `ms`.
fn span_parts(ms: f64) -> [f64; 5] {
    let sign = if ms < 0.0 { -1.0 } else { 1.0 };
    let mut left = ms.abs().round();
    let mut parts = [0.0; 5];
    for (slot, scale) in parts.iter_mut().zip([DAY, HOUR, MINUTE, SECOND, 1.0]) {
        let whole = (left / scale).floor();
        left -= whole * scale;
        *slot = if whole == 0.0 { 0.0 } else { sign * whole };
    }
    parts
}

unsafe fn span_of(g: *mut c_void, v: u64, scale: f64, what: &str) -> Js<u64> {
    unsafe {
        let n = number(g, v)?;
        if !n.is_finite() {
            return range_error(g, &format!("{what}: the value must be a finite number"));
        }
        Ok(span_new(g, n * scale))
    }
}

std_fn!(fn span_from_ms(g, args) { span_of(g, args.arg(0).0, 1.0, "TimeSpan.fromMilliseconds") });
std_fn!(fn span_from_seconds(g, args) { span_of(g, args.arg(0).0, SECOND, "TimeSpan.fromSeconds") });
std_fn!(fn span_from_minutes(g, args) { span_of(g, args.arg(0).0, MINUTE, "TimeSpan.fromMinutes") });
std_fn!(fn span_from_hours(g, args) { span_of(g, args.arg(0).0, HOUR, "TimeSpan.fromHours") });
std_fn!(fn span_from_days(g, args) { span_of(g, args.arg(0).0, DAY, "TimeSpan.fromDays") });

std_fn!(fn span_parse(g, args) {
    let text = string(g, args.arg(0).0, "TimeSpan.parse: the text")?;
    match parse_span(&text) {
        Ok(ms) => Ok(span_new(g, ms)),
        Err(reason) => syntax_error(g, &format!("TimeSpan.parse: \"{text}\" is not a duration: {reason}")),
    }
});

std_fn!(fn span_total_ms(g, args) { Ok(enc_f64(this_span(g, args.this().0)?)) });
std_fn!(fn span_total_seconds(g, args) { Ok(enc_f64(this_span(g, args.this().0)? / SECOND)) });

std_fn!(fn span_add(g, args) {
    let ms = this_span(g, args.this().0)?;
    Ok(span_new(g, ms + ms_arg(g, args.arg(0).0, "TimeSpan.add: the other span")?))
});

std_fn!(fn span_subtract(g, args) {
    let ms = this_span(g, args.this().0)?;
    Ok(span_new(g, ms - ms_arg(g, args.arg(0).0, "TimeSpan.subtract: the other span")?))
});

unsafe fn duration_fields(g: *mut c_void, ms: f64) -> u64 {
    unsafe {
        let fields = new_object(g, UNDEF);
        let names = ["days", "hours", "minutes", "seconds", "milliseconds"];
        for (name, value) in names.iter().zip(span_parts(ms)) {
            define(g, fields, name, enc_f64(value), 0);
        }
        fields
    }
}

std_fn!(fn span_to_duration(g, args) {
    let ms = this_span(g, args.this().0)?;
    let temporal = get(g, glue::rjs_global(g), "Temporal");
    if !is_cell(temporal) {
        return type_error(g, "TimeSpan.toDuration: this host has no Temporal");
    }
    let duration = get(g, temporal, "Duration");
    call(g, get(g, duration, "from"), duration, &[duration_fields(g, ms)]).map_err(|_| Thrown)
});

std_fn!(fn span_to_string(g, args) {
    let ms = this_span(g, args.this().0)?;
    let locale = args.arg(0).0;
    let format = get(g, get(g, glue::rjs_global(g), "Intl"), "DurationFormat");
    if locale == UNDEF || !is_cell(format) {
        return Ok(js_string(g, &span_text(ms)));
    }
    let options = new_object(g, UNDEF);
    define(g, options, "style", js_string(g, "short"), 0);
    let formatter = construct(g, format, &[locale, options])?;
    call(g, get(g, formatter, "format"), formatter, &[duration_fields(g, ms)]).map_err(|_| Thrown)
});

struct Stopwatch {
    banked: Duration,
    started: Option<Instant>,
}

static STOPWATCH: OnceLock<usize> = OnceLock::new();

unsafe fn stopwatch<'a>(g: *mut c_void, this: u64) -> Js<&'a mut Stopwatch> {
    unsafe {
        match host_data::<Stopwatch>(g, host_class::<Stopwatch>(&STOPWATCH, "Stopwatch"), this) {
            Some(state) => Ok(state),
            None => type_error(g, "this is not a Stopwatch"),
        }
    }
}

unsafe fn stopwatch_new(g: *mut c_void, running: bool) -> u64 {
    unsafe {
        let state = Stopwatch {
            banked: Duration::ZERO,
            started: running.then(Instant::now),
        };
        let class = host_class::<Stopwatch>(&STOPWATCH, "Stopwatch");
        host_create(g, class, prototype(g, "Stopwatch"), state)
    }
}

std_fn!(fn stopwatch_construct(g, _args) { Ok(stopwatch_new(g, false)) });
std_fn!(fn stopwatch_start_new(g, _args) { Ok(stopwatch_new(g, true)) });

std_fn!(fn stopwatch_start(g, args) {
    let state = stopwatch(g, args.this().0)?;
    state.started.get_or_insert_with(Instant::now);
    Ok(UNDEF)
});

std_fn!(fn stopwatch_stop(g, args) {
    let state = stopwatch(g, args.this().0)?;
    if let Some(started) = state.started.take() {
        state.banked += started.elapsed();
    }
    Ok(UNDEF)
});

std_fn!(fn stopwatch_reset(g, args) {
    let state = stopwatch(g, args.this().0)?;
    state.banked = Duration::ZERO;
    state.started = None;
    Ok(UNDEF)
});

std_fn!(fn stopwatch_restart(g, args) {
    let state = stopwatch(g, args.this().0)?;
    state.banked = Duration::ZERO;
    state.started = Some(Instant::now());
    Ok(UNDEF)
});

std_fn!(fn stopwatch_elapsed(g, args) {
    let state = stopwatch(g, args.this().0)?;
    let total = state.banked + state.started.map_or(Duration::ZERO, |s| s.elapsed());
    Ok(span_new(g, total.as_secs_f64() * SECOND))
});

std_fn!(fn stopwatch_running(g, args) {
    Ok(enc_bool(stopwatch(g, args.this().0)?.started.is_some()))
});

struct Backoff {
    initial: f64,
    max: f64,
    factor: f64,
    jitter: usize,
    attempt: u32,
}

static BACKOFF: OnceLock<usize> = OnceLock::new();

unsafe fn backoff<'a>(g: *mut c_void, this: u64) -> Js<&'a mut Backoff> {
    unsafe {
        match host_data::<Backoff>(g, host_class::<Backoff>(&BACKOFF, "Backoff"), this) {
            Some(state) => Ok(state),
            None => type_error(g, "this is not a Backoff"),
        }
    }
}

fn backoff_delay(state: &Backoff, random: f64) -> f64 {
    let grown = state.initial * state.factor.powi(state.attempt.min(1024) as i32);
    let delay = grown.min(state.max);
    match state.jitter {
        0 => delay * random,
        1 => delay / 2.0 + delay / 2.0 * random,
        _ => delay,
    }
}

std_fn!(fn backoff_construct(g, args) {
    let options = args.arg(0).0;
    let field = |name: &str, default: f64| -> Js<f64> {
        match option(g, options, name) {
            UNDEF => Ok(default),
            v => ms_arg(g, v, &format!("Backoff: {name}")),
        }
    };
    let initial = field("initial", 100.0)?;
    let max = field("max", 30_000.0)?;
    let factor = match option(g, options, "factor") {
        UNDEF => 2.0,
        v => number(g, v)?,
    };
    if initial < 0.0 || max < initial || factor.is_nan() || factor < 1.0 {
        return range_error(g, "Backoff: initial must not be negative, max not below initial, factor not below 1");
    }
    let jitter = choice(g, option(g, options, "jitter"), &["full", "equal", "none"], 0, "Backoff: jitter")?;
    let state = Backoff { initial, max, factor, jitter, attempt: 0 };
    let class = host_class::<Backoff>(&BACKOFF, "Backoff");
    Ok(host_create(g, class, prototype(g, "Backoff"), state))
});

std_fn!(fn backoff_next(g, args) {
    let this = args.this().0;
    backoff(g, this)?;
    let math = get(g, glue::rjs_global(g), "Math");
    let random = call(g, get(g, math, "random"), math, &[]).map_err(|_| Thrown)?;
    let random = number(g, random)?;
    let state = backoff(g, this)?;
    let delay = backoff_delay(state, random);
    state.attempt = state.attempt.saturating_add(1);
    Ok(enc_f64(delay))
});

std_fn!(fn backoff_reset(g, args) {
    backoff(g, args.this().0)?.attempt = 0;
    Ok(UNDEF)
});

std_fn!(fn backoff_attempt(g, args) {
    Ok(enc_f64(backoff(g, args.this().0)?.attempt as f64))
});

struct Deadline {
    at: Instant,
}

static DEADLINE: OnceLock<usize> = OnceLock::new();

unsafe fn deadline_left(g: *mut c_void, this: u64) -> Js<f64> {
    unsafe {
        match host_data::<Deadline>(g, host_class::<Deadline>(&DEADLINE, "Deadline"), this) {
            Some(state) => Ok(state
                .at
                .saturating_duration_since(Instant::now())
                .as_secs_f64()
                * SECOND),
            None => type_error(g, "this is not a Deadline"),
        }
    }
}

unsafe fn timeout_error(g: *mut c_void) -> Js<u64> {
    unsafe {
        let class = get(g, glue::rjs_global(g), "DOMException");
        construct(
            g,
            class,
            &[
                js_string(g, "The deadline has passed"),
                js_string(g, "TimeoutError"),
            ],
        )
    }
}

std_fn!(fn deadline_after(g, args) {
    let ms = ms_arg(g, args.arg(0).0, "Deadline.after: the time")?;
    let Some(at) = Instant::now().checked_add(Duration::from_secs_f64(ms.max(0.0) / SECOND)) else {
        return range_error(g, "Deadline.after: the time is too far away");
    };
    let class = host_class::<Deadline>(&DEADLINE, "Deadline");
    Ok(host_create(g, class, prototype(g, "Deadline"), Deadline { at }))
});

std_fn!(fn deadline_signal(g, args) {
    let this = args.this().0;
    let left = deadline_left(g, this)?;
    let cached = slot_get(g, this, "skathi.signal");
    if is_cell(cached) {
        return Ok(cached);
    }
    let class = get(g, glue::rjs_global(g), "AbortSignal");
    let signal = if left <= 0.0 {
        call(g, get(g, class, "abort"), class, &[timeout_error(g)?])
    } else {
        call(g, get(g, class, "timeout"), class, &[enc_f64(left.ceil())])
    }
    .map_err(|_| Thrown)?;
    slot_set(g, this, "skathi.signal", signal);
    Ok(signal)
});

std_fn!(fn deadline_remaining(g, args) { Ok(enc_f64(deadline_left(g, args.this().0)?)) });
std_fn!(fn deadline_expired(g, args) { Ok(enc_bool(deadline_left(g, args.this().0)? <= 0.0)) });

std_fn!(fn deadline_throw_if_expired(g, args) {
    if deadline_left(g, args.this().0)? <= 0.0 {
        glue::rjs_throw(g, timeout_error(g)?);
        return Err(Thrown);
    }
    Ok(UNDEF)
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    unsafe {
        let span = support::class(
            g,
            &ClassSpec {
                name: "TimeSpan",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[
                    ("add", 1, span_add),
                    ("subtract", 1, span_subtract),
                    ("toDuration", 0, span_to_duration),
                    ("toString", 0, span_to_string),
                    ("valueOf", 0, span_total_ms),
                    ("toJSON", 0, span_total_ms),
                ],
                getters: &[
                    ("totalMilliseconds", span_total_ms),
                    ("totalSeconds", span_total_seconds),
                ],
                accessors: &[],
            },
        );
        static_fn(g, span.ctor, "fromMilliseconds", 1, span_from_ms);
        static_fn(g, span.ctor, "fromSeconds", 1, span_from_seconds);
        static_fn(g, span.ctor, "fromMinutes", 1, span_from_minutes);
        static_fn(g, span.ctor, "fromHours", 1, span_from_hours);
        static_fn(g, span.ctor, "fromDays", 1, span_from_days);
        static_fn(g, span.ctor, "parse", 1, span_parse);

        let stopwatch = support::class(
            g,
            &ClassSpec {
                name: "Stopwatch",
                arity: 0,
                construct: Some(stopwatch_construct),
                parent_proto: 0,
                methods: &[
                    ("start", 0, stopwatch_start),
                    ("stop", 0, stopwatch_stop),
                    ("reset", 0, stopwatch_reset),
                    ("restart", 0, stopwatch_restart),
                ],
                getters: &[
                    ("elapsed", stopwatch_elapsed),
                    ("running", stopwatch_running),
                ],
                accessors: &[],
            },
        );
        static_fn(g, stopwatch.ctor, "startNew", 0, stopwatch_start_new);

        let backoff = support::class(
            g,
            &ClassSpec {
                name: "Backoff",
                arity: 0,
                construct: Some(backoff_construct),
                parent_proto: 0,
                methods: &[("next", 0, backoff_next), ("reset", 0, backoff_reset)],
                getters: &[("attempt", backoff_attempt)],
                accessors: &[],
            },
        );

        let deadline = support::class(
            g,
            &ClassSpec {
                name: "Deadline",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[
                    ("remaining", 0, deadline_remaining),
                    ("expired", 0, deadline_expired),
                    ("throwIfExpired", 0, deadline_throw_if_expired),
                ],
                getters: &[("signal", deadline_signal)],
                accessors: &[],
            },
        );
        static_fn(g, deadline.ctor, "after", 1, deadline_after);

        let module = new_object(g, UNDEF);
        define(g, module, "TimeSpan", span.ctor, 0);
        define(g, module, "Stopwatch", stopwatch.ctor, 0);
        define(g, module, "Backoff", backoff.ctor, 0);
        define(g, module, "Deadline", deadline.ctor, 0);
        define(g, std, "time", module, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_in_both_notations() {
        assert_eq!(parse_span("1h30m"), Ok(5_400_000.0));
        assert_eq!(parse_span(" 1.5 s "), Ok(1_500.0));
        assert_eq!(parse_span("-250ms"), Ok(-250.0));
        assert_eq!(parse_span("2d 4h"), Ok(2.0 * DAY + 4.0 * HOUR));
        assert_eq!(parse_span("PT1H30M"), Ok(5_400_000.0));
        assert_eq!(parse_span("P1DT0.5S"), Ok(DAY + 500.0));
        assert_eq!(parse_span("P2W"), Ok(14.0 * DAY));
        for bad in ["", "90", "P1M", "1 fortnight", "PT", "h"] {
            assert!(parse_span(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_span_prints_what_parse_reads() {
        for ms in [
            0.0,
            250.0,
            0.5,
            1_500.0,
            5_400_000.0,
            61_001.0,
            -90_061_000.0,
        ] {
            assert_eq!(parse_span(&span_text(ms)), Ok(ms), "{}", span_text(ms));
        }
        assert_eq!(span_text(5_400_000.0), "1h30m");
        assert_eq!(span_text(61_001.0), "1m1.001s");
        assert_eq!(span_parts(-90_061_001.0), [-1.0, -1.0, -1.0, -1.0, -1.0]);
    }

    #[test]
    fn backoff_grows_to_its_cap_and_jitters_inside_it() {
        let mut state = Backoff {
            initial: 100.0,
            max: 1_000.0,
            factor: 2.0,
            jitter: 2,
            attempt: 0,
        };
        let mut delays = Vec::new();
        for _ in 0..5 {
            delays.push(backoff_delay(&state, 0.5));
            state.attempt += 1;
        }
        assert_eq!(delays, [100.0, 200.0, 400.0, 800.0, 1_000.0]);
        state.jitter = 0;
        assert_eq!(backoff_delay(&state, 0.25), 250.0);
        state.jitter = 1;
        assert_eq!(backoff_delay(&state, 0.5), 750.0);
        state.attempt = u32::MAX;
        assert_eq!(backoff_delay(&state, 1.0), 1_000.0);
    }
}
