//! Stateless Web Animations conversion and timing on the VM thread.
use sk_runtime::{
    host_fn,
    redjs::{
        api::{Context, Error, Value},
        glue,
    },
    JsRuntime,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
};

type Result<T> = std::result::Result<T, Value>;
const FILLS: &[&str] = &["none", "forwards", "backwards", "both", "auto"];
const DIRECTIONS: &[&str] = &["normal", "reverse", "alternate", "alternate-reverse"];
const COMPOSITES: &[&str] = &["replace", "add", "accumulate", "auto"];

#[derive(Clone, Copy)]
struct Intrinsics {
    own_keys: Value,
    descriptor: Value,
    has: Value,
}
#[derive(Clone, Copy)]
enum Intrinsic {
    OwnKeys,
    Descriptor,
    Has,
}
thread_local! {
    // JsRuntime and its VM live for the process lifetime.
    static INTRINSICS: Cell<Option<Intrinsics>> = const { Cell::new(None) };
}

/// Each reentrant native call gets its own roots; no mutable registry or Rust callback is held
/// while JS runs.
struct Scope<'a> {
    ctx: &'a Context,
    roots: RefCell<Vec<Value>>,
}
impl<'a> Scope<'a> {
    fn new(ctx: &'a Context) -> Self {
        Self {
            ctx,
            roots: RefCell::new(Vec::new()),
        }
    }
    fn keep(&self, value: Value) -> Value {
        if value.is_cell() {
            self.ctx.protect(value);
            self.roots.borrow_mut().push(value);
        }
        value
    }
    fn checked(&self, status: i32, out: u64) -> Result<Value> {
        match status {
            0 => Ok(self.keep(Value(out))),
            1 => Err(self.keep(Value(out))),
            _ => Err(self.error("Web Animations native conversion failed")),
        }
    }
    fn error(&self, message: &str) -> Value {
        self.keep(self.ctx.error(1, message))
    }
    fn fail<T>(&self, where_: &str, message: &str) -> Result<T> {
        Err(self.error(&format!("Failed to execute '{where_}': {message}")))
    }
    fn get(&self, object: Value, key: &str) -> Result<Value> {
        self.ctx
            .get(object, &self.ctx.atom(key))
            .map(|v| self.keep(v))
            .map_err(|e| self.js_error(e))
    }
    fn get_key(&self, object: Value, key: Value) -> Result<Value> {
        let mut out = 0;
        let status = unsafe { glue::rjs_get_v(self.ctx.raw(), object.0, key.0, &mut out) };
        self.checked(status, out)
    }
    fn index(&self, object: Value, index: u32) -> Result<Value> {
        self.ctx
            .get_index(object, index)
            .map(|v| self.keep(v))
            .map_err(|e| self.js_error(e))
    }
    fn js_error(&self, error: Error) -> Value {
        match error {
            Error::Exception(value) => self.keep(value),
            other => self.error(&other.to_string()),
        }
    }
    fn call(&self, function: Value, receiver: Value, args: &[Value]) -> Result<Value> {
        self.ctx
            .call(function, receiver, args)
            .map(|v| self.keep(v))
            .map_err(|e| self.js_error(e))
    }
    fn builtin(&self, holder: &str, method: &str, args: &[Value]) -> Result<Value> {
        let holder = self
            .ctx
            .global_get(holder)
            .map(|v| self.keep(v))
            .map_err(|e| self.js_error(e))?;
        self.call(self.get(holder, method)?, holder, args)
    }
    fn intrinsic(&self, which: Intrinsic, args: &[Value]) -> Result<Value> {
        let functions = INTRINSICS.with(Cell::get).expect("native WAAPI installed");
        let function = match which {
            Intrinsic::OwnKeys => functions.own_keys,
            Intrinsic::Descriptor => functions.descriptor,
            Intrinsic::Has => functions.has,
        };
        self.call(function, Value::undefined(), args)
    }
    fn convert(&self, name: &str, value: Value) -> Result<Value> {
        let function = self
            .ctx
            .global_get(name)
            .map(|v| self.keep(v))
            .map_err(|e| self.js_error(e))?;
        self.call(function, Value::undefined(), &[value])
    }
    fn number(&self, value: Value) -> Result<f64> {
        // Number(BigInt) succeeds; ToNumber(BigInt) does not.
        self.convert("Number", value)?
            .as_number()
            .ok_or_else(|| self.error("Number conversion did not return a number"))
    }
    fn string(&self, value: Value) -> Result<Value> {
        let value = self.convert("String", value)?;
        if self.ctx.kind(value) != 5 {
            return Err(self.error("String conversion did not return a string"));
        }
        Ok(value)
    }
    fn text(&self, value: Value) -> Result<String> {
        self.ctx.read_string(value).map_err(|e| self.js_error(e))
    }
    fn where_(&self, value: Value) -> Result<String> {
        self.text(self.string(value)?)
    }
    fn truth(&self, value: Value) -> bool {
        unsafe { glue::rjs_to_boolean(self.ctx.raw(), value.0) != 0 }
    }
    fn object(&self, value: Value) -> bool {
        self.ctx.kind(value) >= 8
    }
    fn array(&self, value: Value) -> Result<bool> {
        Ok(self.truth(self.builtin("Array", "isArray", &[value])?))
    }
    fn keys(&self, object: Value) -> Result<Value> {
        let mut out = 0;
        let status = unsafe { glue::rjs_own_keys(self.ctx.raw(), object.0, &mut out) };
        self.checked(status, out)
    }
    fn length(&self, array: Value) -> Result<u32> {
        self.ctx.array_length(array).map_err(|e| self.js_error(e))
    }
    fn new_object(&self) -> Value {
        self.keep(self.ctx.object())
    }
    fn new_array(&self, length: usize) -> Result<Value> {
        let length = u32::try_from(length)
            .map_err(|_| self.error("Web Animations array exceeds the engine limit"))?;
        self.ctx
            .array(length)
            .map(|v| self.keep(v))
            .map_err(|e| self.js_error(e))
    }
    fn put(&self, object: Value, key: &str, value: Value) -> Result<()> {
        // Own data properties, never a user prototype setter on a fresh result.
        self.ctx
            .define(object, &self.ctx.atom(key), value, 0)
            .map_err(|e| self.js_error(e))
    }
    fn put_key(&self, object: Value, key: Value, value: Value) -> Result<()> {
        let status = unsafe { glue::rjs_define_v(self.ctx.raw(), object.0, key.0, value.0) };
        if status == 0 {
            Ok(())
        } else {
            Err(self.error("Web Animations result property could not be defined"))
        }
    }
    fn put_index(&self, array: Value, index: usize, value: Value) -> Result<()> {
        let index = u32::try_from(index)
            .map_err(|_| self.error("Web Animations array exceeds the engine limit"))?;
        let status = unsafe { glue::rjs_define_index(self.ctx.raw(), array.0, index, value.0) };
        if status == 0 {
            Ok(())
        } else {
            Err(self.error("Web Animations result index could not be defined"))
        }
    }
    fn literal(&self, text: &str) -> Value {
        self.keep(self.ctx.string(text))
    }
    fn iterator_method(&self, value: Value) -> Result<Value> {
        let mut out = 0;
        let status = unsafe {
            let atom = glue::rjs_atom_iterator(self.ctx.raw());
            let status = glue::rjs_get(self.ctx.raw(), value.0, atom, &mut out);
            glue::rjs_atom_free(self.ctx.raw(), atom);
            status
        };
        self.checked(status, out)
    }
    fn close_iterator(&self, iterator: Value) {
        if let Ok(method) = self.get(iterator, "return") {
            if !method.is_null() && !method.is_undefined() {
                let _closed = self.call(method, iterator, &[]);
            }
        }
    }
    fn trim(&self, text: Value) -> Result<Value> {
        self.call(self.get(text, "trim")?, text, &[])
    }
    fn easing(&self, value: Value, where_: &str) -> Result<Value> {
        if value.is_undefined() {
            return Ok(self.literal("linear"));
        }
        let text = self.string(value)?;
        let trimmed = self.trim(text)?;
        if sk_css::parse_easing(&self.text(trimmed)?).is_none() {
            return self.fail(
                where_,
                &format!("'{}' is not a valid value for easing", self.text(text)?),
            );
        }
        Ok(trimmed)
    }
    fn composite(&self, value: Value, where_: &str) -> Result<Value> {
        if value.is_null() || value.is_undefined() {
            return Ok(self.literal("auto"));
        }
        self.enumeration(value, COMPOSITES, "composite", where_)
    }
    fn enumeration(
        &self,
        value: Value,
        choices: &[&str],
        name: &str,
        where_: &str,
    ) -> Result<Value> {
        let string = self.string(value)?;
        let text = self.text(string)?;
        if !choices.contains(&text.as_str()) {
            return self.fail(
                where_,
                &format!("'{text}' is not a valid value for {name}."),
            );
        }
        Ok(string)
    }
    fn offset(&self, value: Value, where_: &str) -> Result<Option<f64>> {
        if value.is_null() || value.is_undefined() {
            return Ok(None);
        }
        let number = self.number(value)?;
        if !number.is_finite() {
            return self.fail(where_, "Non-finite offset.");
        }
        Ok(Some(number))
    }
    /// Array iteration keeps holes and observes each coercion before reading the next element, so
    /// getters can change later indexed values.
    fn each_list(
        &self,
        value: Value,
        optional: bool,
        mapped: bool,
        mut each: impl FnMut(usize, Value) -> Result<()>,
    ) -> Result<usize> {
        if optional && value.is_undefined() {
            return Ok(0);
        }
        if !self.array(value)? {
            each(0, value)?;
            return Ok(1);
        }
        let length_value = self.get(value, "length")?;
        let mut length = 0.0;
        let mut exception = 0;
        let status = unsafe {
            glue::rjs_to_number_x(self.ctx.raw(), length_value.0, &mut length, &mut exception)
        };
        if status != 0 {
            return Err(self.keep(Value(exception)));
        }
        let length = if length.is_nan() || length <= 0.0 {
            0
        } else {
            length.floor().min(9_007_199_254_740_991.0) as usize
        };
        if mapped && length > u32::MAX as usize {
            return Err(self.keep(self.ctx.error(2, "Invalid array length")));
        }
        for index in 0..length {
            // Only converted outputs belong to the outer scope.
            let transient = Scope::new(self.ctx);
            let key = transient.literal(&index.to_string());
            let exists = transient.intrinsic(Intrinsic::Has, &[value, key])?;
            if transient.truth(exists) {
                each(index, transient.get_key(value, key)?)?;
            }
        }
        Ok(length)
    }
    fn clone_record(&self, source: Value) -> Result<Value> {
        let result = self.new_object();
        if source.is_null() || source.is_undefined() {
            return Ok(result);
        }
        let keys = self.intrinsic(Intrinsic::OwnKeys, &[source])?;
        for index in 0..self.length(keys)? {
            let key = self.index(keys, index)?;
            let descriptor = self.intrinsic(Intrinsic::Descriptor, &[source, key])?;
            if descriptor.is_undefined() || !self.truth(self.get(descriptor, "enumerable")?) {
                continue;
            }
            self.put_key(result, key, self.get_key(source, key)?)?;
        }
        Ok(result)
    }
}
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        for value in self.roots.get_mut().drain(..) {
            self.ctx.unprotect(value);
        }
    }
}

struct Frame {
    offset: Option<f64>,
    computed: f64,
    easing: Value,
    composite: Value,
    props: Vec<(Value, Value)>,
}
fn reserved(name: &str) -> bool {
    matches!(name, "offset" | "easing" | "composite")
}
fn check_offsets(scope: &Scope<'_>, frames: &[Frame], where_: &str) -> Result<()> {
    let mut previous = f64::NEG_INFINITY;
    for offset in frames.iter().filter_map(|frame| frame.offset) {
        if !(0.0..=1.0).contains(&offset) {
            return scope.fail(where_, "Offsets must be null or in the range [0,1].");
        }
        if offset < previous {
            return scope.fail(where_, "Offsets must be monotonically non-decreasing.");
        }
        previous = offset;
    }
    Ok(())
}
fn compute_offsets(frames: &mut [Frame]) {
    if frames.is_empty() {
        return;
    }
    for frame in frames.iter_mut() {
        frame.computed = frame.offset.unwrap_or(f64::NAN);
    }
    if frames.len() > 1 && frames[0].offset.is_none() {
        frames[0].computed = 0.0;
    }
    let last = frames.len() - 1;
    if frames[last].offset.is_none() {
        frames[last].computed = 1.0;
    }
    let mut previous = 0;
    for index in 1..frames.len() {
        if frames[index].computed.is_nan() {
            continue;
        }
        for missing in previous + 1..index {
            frames[missing].computed = frames[previous].computed
                + (frames[index].computed - frames[previous].computed)
                    * (missing - previous) as f64
                    / (index - previous) as f64;
        }
        previous = index;
    }
}
fn list_frame(scope: &Scope<'_>, value: Value, where_: &str) -> Result<Frame> {
    let value = if value.is_null() || value.is_undefined() {
        scope.new_object()
    } else {
        value
    };
    if !scope.object(value) || scope.ctx.kind(value) == 9 {
        return scope.fail(where_, "Keyframes must be objects.");
    }
    let keys = scope.keys(value)?;
    let mut props = Vec::new();
    for index in 0..scope.length(keys)? {
        let key = scope.index(keys, index)?;
        if !reserved(&scope.text(key)?) {
            props.push((key, scope.string(scope.get_key(value, key)?)?));
        }
    }
    let offset = scope.offset(scope.get(value, "offset")?, where_)?;
    let easing = scope.easing(scope.get(value, "easing")?, where_)?;
    let composite = scope.composite(scope.get(value, "composite")?, where_)?;
    Ok(Frame {
        offset,
        computed: 0.0,
        easing,
        composite,
        props,
    })
}
fn keyframes(scope: &Scope<'_>, input: Value, where_: &str) -> Result<Value> {
    let mut frames = Vec::new();
    if !input.is_null() && !input.is_undefined() {
        if !scope.object(input) {
            return scope.fail(
                where_,
                "The provided value is not of type '(sequence<object> or object)'.",
            );
        }
        if scope.ctx.kind(scope.iterator_method(input)?) == 9 {
            let iterator = scope.call(scope.iterator_method(input)?, input, &[])?;
            if !scope.object(iterator) {
                return scope.fail(where_, "Keyframe iterator must return an object.");
            }
            let next = scope.get(iterator, "next")?;
            loop {
                let step = scope.call(next, iterator, &[])?;
                if !scope.object(step) {
                    return scope.fail(where_, "Keyframe iterator result must be an object.");
                }
                if scope.truth(scope.get(step, "done")?) {
                    break;
                }
                let value = scope.get(step, "value")?;
                match list_frame(scope, value, where_) {
                    Ok(frame) => frames.push(frame),
                    Err(error) => {
                        scope.close_iterator(iterator);
                        return Err(error);
                    }
                }
            }
            check_offsets(scope, &frames, where_)?;
            compute_offsets(&mut frames);
        } else {
            let keys = scope.keys(input)?;
            for index in 0..scope.length(keys)? {
                let key = scope.index(keys, index)?;
                if reserved(&scope.text(key)?) {
                    continue;
                }
                let raw = scope.get_key(input, key)?;
                if raw.is_undefined() {
                    continue;
                }
                let mut strings = Vec::new();
                let length = scope.each_list(raw, false, true, |index, value| {
                    strings.push((index, scope.string(value)?));
                    Ok(())
                })?;
                for (position, value) in strings {
                    let at = if length == 1 {
                        1.0
                    } else {
                        position as f64 / (length - 1) as f64
                    };
                    let frame_index = if let Some(found) =
                        frames.iter().position(|frame: &Frame| frame.computed == at)
                    {
                        found
                    } else {
                        frames.push(Frame {
                            offset: None,
                            computed: at,
                            easing: scope.literal("linear"),
                            composite: scope.literal("auto"),
                            props: Vec::new(),
                        });
                        frames.len() - 1
                    };
                    frames[frame_index].props.push((key, value));
                }
            }
            frames.sort_by(|a, b| a.computed.total_cmp(&b.computed));
            scope.each_list(scope.get(input, "offset")?, true, false, |index, value| {
                if index < frames.len() {
                    frames[index].offset = scope.offset(value, where_)?;
                }
                Ok(())
            })?;
            let mut easings = Vec::new();
            let length =
                scope.each_list(scope.get(input, "easing")?, true, true, |index, value| {
                    easings.push((index, scope.easing(value, where_)?));
                    Ok(())
                })?;
            if length != 0 {
                for (index, frame) in frames.iter_mut().enumerate() {
                    frame.easing = easings
                        .iter()
                        .find(|(at, _)| *at == index % length)
                        .map(|(_, value)| *value)
                        .unwrap_or(Value::undefined());
                }
            }
            let mut composites = Vec::new();
            let length = scope.each_list(
                scope.get(input, "composite")?,
                true,
                true,
                |index, value| {
                    composites.push((index, scope.composite(value, where_)?));
                    Ok(())
                },
            )?;
            if length != 0 {
                for (index, frame) in frames.iter_mut().enumerate() {
                    frame.composite = composites
                        .iter()
                        .find(|(at, _)| *at == index % length)
                        .map(|(_, value)| *value)
                        .unwrap_or(Value::undefined());
                }
            }
            check_offsets(scope, &frames, where_)?;
            for frame in &mut frames {
                if let Some(offset) = frame.offset {
                    frame.computed = offset;
                }
            }
        }
    }
    let output = scope.new_array(frames.len())?;
    for (index, frame) in frames.into_iter().enumerate() {
        let value = scope.new_object();
        scope.put(
            value,
            "offset",
            frame.offset.map(Value::number).unwrap_or(Value::null()),
        )?;
        scope.put(value, "computed", Value::number(frame.computed))?;
        scope.put(value, "easing", frame.easing)?;
        scope.put(value, "composite", frame.composite)?;
        let props = scope.new_array(frame.props.len())?;
        for (index, (key, val)) in frame.props.into_iter().enumerate() {
            let pair = scope.new_array(2)?;
            scope.put_index(pair, 0, key)?;
            scope.put_index(pair, 1, val)?;
            scope.put_index(props, index, pair)?;
        }
        scope.put(value, "props", props)?;
        scope.put_index(output, index, value)?;
    }
    Ok(output)
}

fn timing(scope: &Scope<'_>, base: Value, input: Value, where_: &str) -> Result<Value> {
    let result = scope.clone_record(base)?;
    if input.is_null() || input.is_undefined() {
        return Ok(result);
    }
    if matches!(scope.ctx.kind(input), 3 | 4) {
        let duration = input.as_number().unwrap_or(f64::NAN);
        if duration.is_nan() || duration < 0.0 {
            return scope.fail(where_, "duration must be non-negative or auto.");
        }
        scope.put(result, "duration", input)?;
        return Ok(result);
    }
    if !scope.object(input) || scope.ctx.kind(input) == 9 {
        return Ok(result);
    }
    for name in [
        "delay",
        "endDelay",
        "fill",
        "iterationStart",
        "iterations",
        "duration",
        "direction",
        "easing",
    ] {
        if scope.get(input, name)?.is_undefined() {
            continue;
        }
        let raw = scope.get(input, name)?;
        let value = match name {
            "fill" => scope.enumeration(raw, FILLS, name, where_)?,
            "direction" => scope.enumeration(raw, DIRECTIONS, name, where_)?,
            "easing" => scope.easing(raw, where_)?,
            "duration" if scope.ctx.kind(raw) == 5 => {
                let checked = scope.get(input, "duration")?;
                if scope.ctx.kind(checked) != 5 || scope.text(checked)? != "auto" {
                    return scope.fail(where_, "duration must be non-negative or auto.");
                }
                scope.literal("auto")
            }
            _ => {
                let number = scope.number(if name == "duration" {
                    scope.get(input, name)?
                } else {
                    raw
                })?;
                if matches!(name, "delay" | "endDelay" | "iterationStart") && !number.is_finite() {
                    return scope.fail(where_, &format!("{name} must be a finite number."));
                }
                if name == "iterationStart" && number < 0.0 {
                    return scope.fail(where_, "iterationStart must be non-negative.");
                }
                if matches!(name, "iterations" | "duration") && (number.is_nan() || number < 0.0) {
                    let message = if name == "duration" {
                        "duration must be non-negative or auto."
                    } else {
                        "iterations must be non-negative."
                    };
                    return scope.fail(where_, message);
                }
                Value::number(number)
            }
        };
        scope.put(result, name, value)?;
    }
    Ok(result)
}
fn resolved(scope: &Scope<'_>, object: Value) -> Result<sk_css::ScriptTiming> {
    let number = |name| {
        scope
            .get(object, name)?
            .as_number()
            .ok_or_else(|| scope.error(&format!("normalized timing {name} is not a number")))
    };
    let duration = scope.get(object, "duration")?;
    let duration = if scope.ctx.kind(duration) == 5 && scope.text(duration)? == "auto" {
        0.0
    } else {
        duration
            .as_number()
            .ok_or_else(|| scope.error("normalized duration is not a number"))?
    };
    let fill = match scope.text(scope.get(object, "fill")?)?.as_str() {
        "none" | "auto" => sk_css::FillMode::None,
        "forwards" => sk_css::FillMode::Forwards,
        "backwards" => sk_css::FillMode::Backwards,
        "both" => sk_css::FillMode::Both,
        _ => return Err(scope.error("invalid normalized fill")),
    };
    let direction = match scope.text(scope.get(object, "direction")?)?.as_str() {
        "normal" => sk_css::AnimationDirection::Normal,
        "reverse" => sk_css::AnimationDirection::Reverse,
        "alternate" => sk_css::AnimationDirection::Alternate,
        "alternate-reverse" => sk_css::AnimationDirection::AlternateReverse,
        _ => return Err(scope.error("invalid normalized direction")),
    };
    let easing_text = scope.text(scope.get(object, "easing")?)?;
    let easing = sk_css::parse_easing(&easing_text)
        .ok_or_else(|| scope.error("invalid normalized easing"))?;
    Ok(sk_css::ScriptTiming {
        delay: number("delay")?,
        end_delay: number("endDelay")?,
        duration,
        iterations: number("iterations")?,
        iteration_start: number("iterationStart")?,
        direction,
        fill,
        easing,
    })
}
fn local_time(scope: &Scope<'_>, value: Value) -> Result<Option<f64>> {
    if value.is_null() || value.is_undefined() {
        Ok(None)
    } else {
        value
            .as_number()
            .map(Some)
            .ok_or_else(|| scope.error("local time must be a number or null"))
    }
}
fn computed(scope: &Scope<'_>, timing: Value, local: Value, backwards: Value) -> Result<Value> {
    let timing = resolved(scope, timing)?;
    let local = local_time(scope, local)?;
    let sampled = timing.sample(local, scope.truth(backwards));
    let result = scope.new_object();
    let fill = match timing.fill {
        sk_css::FillMode::None => "none",
        sk_css::FillMode::Forwards => "forwards",
        sk_css::FillMode::Backwards => "backwards",
        sk_css::FillMode::Both => "both",
    };
    scope.put(result, "fill", scope.literal(fill))?;
    for (key, value) in [
        ("duration", Some(timing.duration)),
        ("endTime", Some(timing.end_time())),
        ("activeDuration", Some(timing.active_duration())),
        ("localTime", local),
        ("progress", sampled.progress),
        ("currentIteration", sampled.current_iteration),
    ] {
        scope.put(
            result,
            key,
            value.map(Value::number).unwrap_or(Value::null()),
        )?;
    }
    Ok(result)
}
fn run(g: *mut c_void, body: impl FnOnce(&Scope<'_>) -> Result<Value>) -> u64 {
    JsRuntime::with(|runtime| {
        let scope = Scope::new(runtime.ctx());
        match body(&scope) {
            Ok(value) => value.0,
            Err(error) => {
                unsafe { glue::rjs_throw(g, error.0) };
                Value::undefined().0
            }
        }
    })
}
host_fn!(fn native_keyframes(g, args) { run(g, |s| keyframes(s, args.arg(0), &s.where_(args.arg(1))?)) });
host_fn!(fn native_timing(g, args) { run(g, |s| timing(s, args.arg(0), args.arg(1), &s.where_(args.arg(2))?)) });
host_fn!(fn native_computed(g, args) { run(g, |s| computed(s, args.arg(0), args.arg(1), args.arg(2))) });
host_fn!(fn native_end_time(g, args) { run(g, |s| Ok(Value::number(resolved(s, args.arg(0))?.end_time()))) });
host_fn!(fn native_in_effect(g, args) { run(g, |s| Ok(Value::boolean(resolved(s, args.arg(0))?.sample(local_time(s, args.arg(1))?, s.truth(args.arg(2))).progress.is_some()))) });
host_fn!(fn native_valid_easing(g, args) { run(g, |s| {
    let text = s.trim(s.string(args.arg(0))?)?;
    Ok(Value::boolean(sk_css::parse_easing(&s.text(text)?).is_some()))
}) });
host_fn!(fn native_evaluate_easing(g, args) { run(g, |s| {
    let text = s.trim(s.string(args.arg(0))?)?;
    let easing = sk_css::parse_easing(&s.text(text)?).ok_or_else(|| s.error("invalid easing"))?;
    Ok(Value::number(easing.eval_f64(s.number(args.arg(1))?)))
}) });

fn css_name(idl: &str) -> Option<String> {
    if idl.starts_with("--") {
        return Some(idl.to_string());
    }
    match idl {
        "cssFloat" => return Some("float".into()),
        "cssOffset" => return Some("offset".into()),
        _ => {}
    }
    if idl.contains('-') {
        return None;
    }
    let mut css = String::new();
    for character in idl.chars() {
        if character.is_ascii_uppercase() {
            css.push('-');
            css.push(character.to_ascii_lowercase());
        } else {
            css.push(character);
        }
    }
    if css.starts_with("webkit-") {
        css.insert(0, '-');
    }
    Some(css)
}
fn strings(scope: &Scope<'_>, values: &[String]) -> Result<Value> {
    let result = scope.new_array(values.len())?;
    for (index, value) in values.iter().enumerate() {
        scope.put_index(result, index, scope.literal(value))?;
    }
    Ok(result)
}
fn effect_notes(
    scope: &Scope<'_>,
    accepted: bool,
    error: Option<&str>,
    ignored: &[String],
    diagnostics: &[String],
    hyphenated: &[Value],
) -> Result<Value> {
    let result = scope.new_object();
    scope.put(result, "accepted", Value::boolean(accepted))?;
    scope.put(
        result,
        "error",
        error
            .map(|text| scope.literal(text))
            .unwrap_or(Value::null()),
    )?;
    scope.put(result, "ignored", strings(scope, ignored)?)?;
    scope.put(result, "diagnostics", strings(scope, diagnostics)?)?;
    let names = scope.new_array(hyphenated.len())?;
    for (index, value) in hyphenated.iter().enumerate() {
        scope.put_index(names, index, *value)?;
    }
    scope.put(result, "hyphenated", names)?;
    Ok(result)
}
fn set_effect(
    scope: &Scope<'_>,
    id: Value,
    node: Value,
    frames: Value,
    timing: Value,
) -> Result<Value> {
    // All user reads/coercions finish before the retained tree/animator borrows.
    let timing = resolved(scope, timing)?;
    let mut parsed = Vec::new();
    let mut hyphenated = Vec::new();
    for index in 0..scope.length(frames)? {
        let frame = scope.index(frames, index)?;
        let offset = scope
            .get(frame, "computed")?
            .as_number()
            .ok_or_else(|| scope.error("normalized keyframe offset is not a number"))?;
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) {
            return effect_notes(
                scope,
                false,
                Some("invalid keyframe offset"),
                &[],
                &[],
                &hyphenated,
            );
        }
        let easing = scope.text(scope.string(scope.get(frame, "easing")?)?)?;
        let easing = match easing.trim() {
            "" | "linear" => None,
            text => match sk_css::parse_easing(text) {
                Some(easing) => Some(easing),
                None => {
                    return effect_notes(
                        scope,
                        false,
                        Some(&format!("'{text}' is not a valid easing")),
                        &[],
                        &[],
                        &hyphenated,
                    )
                }
            },
        };
        let props = scope.get(frame, "props")?;
        let mut decls = Vec::new();
        for index in 0..scope.length(props)? {
            let pair = scope.index(props, index)?;
            let key = scope.string(scope.index(pair, 0)?)?;
            let idl = scope.text(key)?;
            let Some(css) = css_name(&idl) else {
                if !hyphenated.iter().any(|previous: &Value| unsafe {
                    glue::rjs_strict_equals(scope.ctx.raw(), previous.0, key.0) != 0
                }) {
                    hyphenated.push(key);
                }
                continue;
            };
            let value = scope.text(scope.string(scope.index(pair, 1)?)?)?;
            decls.push((css, value));
        }
        parsed.push(sk_css::ScriptFrame {
            offset: offset as f32,
            easing,
            decls,
        });
    }
    let notes = crate::tree::script_anim_set(
        id.as_number().unwrap_or(0.0) as u32,
        node.as_number().unwrap_or(0.0) as u32,
        &parsed,
        timing,
    );
    effect_notes(
        scope,
        true,
        None,
        &notes.ignored,
        &notes.diagnostics,
        &hyphenated,
    )
}
fn values(scope: &Scope<'_>, id: Value) -> Result<Value> {
    let Some(values) = crate::tree::script_anim_values(id.as_number().unwrap_or(0.0) as u32) else {
        return Ok(Value::null());
    };
    let result = scope.new_array(values.len())?;
    for (index, (key, value)) in values.into_iter().enumerate() {
        let pair = scope.new_array(2)?;
        scope.put_index(pair, 0, scope.literal(&key))?;
        scope.put_index(pair, 1, scope.literal(&value))?;
        scope.put_index(result, index, pair)?;
    }
    Ok(result)
}
host_fn!(fn native_set_effect(g, args) { run(g, |s| set_effect(s, args.arg(0), args.arg(1), args.arg(2), args.arg(3))) });
host_fn!(fn native_clock(_g, args) {
    let number = |index| args.arg(index).as_number().filter(|value| !value.is_nan());
    crate::tree::script_anim_clock(args.arg(0).as_number().unwrap_or(0.0) as u32, sk_css::ScriptClock {
        start: number(1), hold: number(2), rate: number(3).unwrap_or(1.0),
    });
    Value::undefined().0
});
host_fn!(fn native_drop(_g, args) {
    crate::tree::script_anim_drop(args.arg(0).as_number().unwrap_or(0.0) as u32);
    Value::undefined().0
});
host_fn!(fn native_values(g, args) { run(g, |s| values(s, args.arg(0))) });

pub(crate) fn install(runtime: &JsRuntime) {
    let scope = Scope::new(runtime.ctx());
    if INTRINSICS.with(Cell::get).is_none() {
        let reflect = scope.keep(scope.ctx.global_get("Reflect").expect("Reflect intrinsic"));
        let object = scope.keep(scope.ctx.global_get("Object").expect("Object intrinsic"));
        let functions = Intrinsics {
            own_keys: scope
                .get(reflect, "ownKeys")
                .expect("Reflect.ownKeys intrinsic"),
            descriptor: scope
                .get(object, "getOwnPropertyDescriptor")
                .expect("Object.getOwnPropertyDescriptor intrinsic"),
            has: scope.get(reflect, "has").expect("Reflect.has intrinsic"),
        };
        for function in [functions.own_keys, functions.descriptor, functions.has] {
            scope.ctx.protect(function);
        }
        INTRINSICS.with(|slot| slot.set(Some(functions)));
    }
    let namespace = scope.new_object();
    for (name, arity, function) in [
        ("keyframes", 2, native_keyframes as *const c_void),
        ("timing", 3, native_timing as *const c_void),
        ("computed", 3, native_computed as *const c_void),
        ("endTime", 1, native_end_time as *const c_void),
        ("inEffect", 3, native_in_effect as *const c_void),
        ("validEasing", 1, native_valid_easing as *const c_void),
        ("evaluateEasing", 2, native_evaluate_easing as *const c_void),
        ("setEffect", 4, native_set_effect as *const c_void),
        ("clock", 4, native_clock as *const c_void),
        ("drop", 1, native_drop as *const c_void),
        ("values", 1, native_values as *const c_void),
    ] {
        let method = unsafe { scope.ctx.function_sysv(name, arity, function) }
            .unwrap_or_else(|error| panic!("installing native WAAPI {name}: {error}"));
        scope.keep(method);
        scope
            .put(namespace, name, method)
            .unwrap_or_else(|_| panic!("installing native WAAPI {name}"));
    }
    scope
        .ctx
        .global_set("__sk_waapi_core", namespace)
        .expect("installing native WAAPI namespace");
}

#[cfg(test)]
#[path = "waapi_tests.rs"]
mod tests;
