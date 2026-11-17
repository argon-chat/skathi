//! Native module values, read from the engine straight into serde types and built from serde
//! output straight into engine values, meaning what a JSON round trip (`JSON.stringify`, then
//! serde_json; serde_json, then `JSON.parse`) means.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use redjs::api::Atom;
use redjs::glue;
use serde::de::{self, DeserializeSeed, IntoDeserializer, Unexpected, Visitor};
use serde::ser::{self, Serialize};
use serde_json::Value as Json;

use crate::js::{Context, Value};

#[derive(Debug)]
pub(crate) enum Error {
    /// What serde reports; the caller prefixes the argument.
    Message(String),
    /// A JS exception (a getter, `toJSON`, a BigInt, a cycle), rethrown as it is.
    Thrown(Value),
    /// What serde_json refused to parse in the JSON round trip.
    NotJson(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Message(m) | Error::NotJson(m) => f.write_str(m),
            Error::Thrown(_) => f.write_str("a JS exception"),
        }
    }
}

impl std::error::Error for Error {}

impl de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }

    // serde_json words these its own way ("null", its float format).
    fn invalid_type(unexp: Unexpected<'_>, exp: &dyn de::Expected) -> Self {
        Error::Message(<serde_json::Error as de::Error>::invalid_type(unexp, exp).to_string())
    }

    fn invalid_value(unexp: Unexpected<'_>, exp: &dyn de::Expected) -> Self {
        Error::Message(<serde_json::Error as de::Error>::invalid_value(unexp, exp).to_string())
    }
}

impl ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}

fn not_json(message: impl fmt::Display) -> Error {
    Error::NotJson(format!("the arguments are not JSON: {message}"))
}

fn engine(e: crate::js::Error) -> Error {
    match e {
        crate::js::Error::Exception(v) => Error::Thrown(v),
        other => Error::Message(other.to_string()),
    }
}

/// What the realm had at install, protected for good.
#[derive(Clone, Copy)]
struct Intrinsics {
    parse: Value,
    stringify: Value,
    is_array: Value,
}

thread_local! {
    static INTRINSICS: std::cell::Cell<Option<Intrinsics>> = const { std::cell::Cell::new(None) };
    static ATOMS: RefCell<HashMap<(usize, usize), Atom>> = RefCell::new(HashMap::new());
}

pub(crate) fn install(context: &Context) -> crate::js::Result<()> {
    let json = context.global_get("JSON")?;
    let array = context.global_get("Array")?;
    let intrinsics = Intrinsics {
        parse: context.get(json, &context.atom("parse"))?,
        stringify: context.get(json, &context.atom("stringify"))?,
        is_array: context.get(array, &context.atom("isArray"))?,
    };
    context.protect(intrinsics.parse);
    context.protect(intrinsics.stringify);
    context.protect(intrinsics.is_array);
    INTRINSICS.with(|i| i.set(Some(intrinsics)));
    Ok(())
}

fn intrinsics() -> Result<Intrinsics, Error> {
    INTRINSICS
        .with(|i| i.get())
        .ok_or_else(|| Error::Message("native modules are not installed".into()))
}

/// A property name made once per field name and kept.
fn with_atom<T>(context: &Context, name: &'static str, f: impl FnOnce(&Atom) -> T) -> T {
    ATOMS.with(|atoms| {
        let mut atoms = atoms.borrow_mut();
        let atom = atoms
            .entry((name.as_ptr() as usize, name.len()))
            .or_insert_with(|| context.atom(name));
        f(atom)
    })
}

pub(crate) fn parse(context: &Context, text: &str) -> Result<Value, Error> {
    let parse = intrinsics()?.parse;
    context
        .call(parse, Value::undefined(), &[string(context, text)])
        .map_err(engine)
}

/// `JSON.stringify(value)`, for its exception: a BigInt's or a cycle's TypeError, word for word.
fn stringify_error(context: &Context, value: Value) -> Error {
    let stringify = match intrinsics() {
        Ok(i) => i.stringify,
        Err(e) => return e,
    };
    match context.call(stringify, Value::undefined(), &[value]) {
        Err(e) => engine(e),
        Ok(_) => Error::Message("JSON.stringify accepted what it refused before".into()),
    }
}

/// What JSON.stringify makes of `value` as an array element, read back by serde_json.
fn through_json(context: &Context, value: Value) -> Result<Json, Error> {
    let stringify = intrinsics()?.stringify;
    let array = context.array(1).map_err(engine)?;
    context.set_index(array, 0, value).map_err(engine)?;
    let text = context
        .call(stringify, Value::undefined(), &[array])
        .map_err(engine)?;
    let text = context.read_string(text).map_err(engine)?;
    let mut list: Vec<Json> = serde_json::from_str(&text).map_err(|e| {
        let e = e.to_string();
        not_json(e.split(" at line ").next().unwrap_or_default())
    })?;
    Ok(list.pop().unwrap_or(Json::Null))
}

pub(crate) fn string(context: &Context, text: &str) -> Value {
    if text.is_ascii() {
        let mut out = 0u64;
        // SAFETY: the bytes are read during the call.
        let status =
            unsafe { glue::rjs_string_latin1(context.raw(), text.as_ptr(), text.len(), &mut out) };
        if status == glue::OK {
            return Value(out);
        }
    }
    context.string(text)
}

/// A JS string's text; ASCII is borrowed from the engine's own storage. `None`: not a string.
///
/// # Safety
/// The result borrows the string cell: use it while `value` is alive on the stack.
unsafe fn read_str<'s>(context: &Context, value: Value) -> Option<Result<Cow<'s, str>, Error>> {
    let mut info = glue::RjsStr::default();
    if unsafe { glue::rjs_string_info(context.raw(), value.0, &mut info) } != glue::OK {
        return None;
    }
    if info.len == 0 {
        return Some(Ok(Cow::Borrowed("")));
    }
    let len = info.len as usize;
    Some(if info.is8bit != 0 {
        let bytes = unsafe { std::slice::from_raw_parts(info.ptr as *const u8, len) };
        if bytes.is_ascii() {
            Ok(Cow::Borrowed(unsafe {
                std::str::from_utf8_unchecked(bytes)
            }))
        } else {
            Ok(Cow::Owned(bytes.iter().map(|&b| b as char).collect()))
        }
    } else {
        let units = unsafe { std::slice::from_raw_parts(info.ptr as *const u16, len) };
        from_utf16(units).map(Cow::Owned)
    })
}

/// Lone surrogates fail as serde_json fails on the `\uXXXX` escapes JSON.stringify writes for them.
fn from_utf16(units: &[u16]) -> Result<String, Error> {
    let mut out = String::with_capacity(units.len());
    let mut at = 0;
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok(c) => {
                out.push(c);
                at += c.len_utf16();
            }
            Err(e) => {
                let unit = e.unpaired_surrogate();
                let message = if (0xDC00..=0xDFFF).contains(&unit) {
                    "lone leading surrogate in hex escape"
                } else {
                    match units.get(at + 1) {
                        Some(&next) if escaped_as_hex(next, units.get(at + 2)) => {
                            "lone leading surrogate in hex escape"
                        }
                        _ => "unexpected end of hex escape",
                    }
                };
                return Err(not_json(message));
            }
        }
    }
    Ok(out)
}

/// Whether JSON.stringify writes this unit as `\uXXXX`.
fn escaped_as_hex(unit: u16, next: Option<&u16>) -> bool {
    match unit {
        0x08 | 0x09 | 0x0A | 0x0C | 0x0D => false,
        0..=0x1F => true,
        0xD800..=0xDBFF => !next.is_some_and(|n| (0xDC00..=0xDFFF).contains(n)),
        0xDC00..=0xDFFF => true,
        _ => false,
    }
}

/// How serde_json reads the digits JSON.stringify writes for a finite number.
fn visit_number<'de, V: Visitor<'de>>(n: f64, visitor: V) -> Result<V::Value, Error> {
    match number(n) {
        Number::Unsigned(u) => visitor.visit_u64(u),
        Number::Signed(i) => visitor.visit_i64(i),
        Number::Float(f) => visitor.visit_f64(f),
    }
}

fn number_unexpected(n: f64) -> Unexpected<'static> {
    match number(n) {
        Number::Unsigned(u) => Unexpected::Unsigned(u),
        Number::Signed(i) => Unexpected::Signed(i),
        Number::Float(f) => Unexpected::Float(f),
    }
}

enum Number {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
}

const TWO_53: f64 = 9_007_199_254_740_992.0;

#[inline]
fn number(n: f64) -> Number {
    if n.abs() >= TWO_53 {
        return beyond_2_53(n);
    }
    if n.fract() != 0.0 {
        Number::Float(n)
    } else if n >= 0.0 {
        Number::Unsigned(n as u64)
    } else {
        Number::Signed(n as i64)
    }
}

/// From 2^53 to 1e21 JSON.stringify writes the shortest digits padded with zeros, so 2^63
/// reached serde_json as `9223372036854776000`: an integer, but not the number itself.
#[cold]
fn beyond_2_53(n: f64) -> Number {
    if n.abs() >= 1e21 {
        return Number::Float(n);
    }
    let mut buffer = zmij::Buffer::new();
    let shortest = buffer.format_finite(n.abs());
    let (mantissa, exponent) = match shortest.split_once('e') {
        Some((m, e)) => (m, e.parse::<i32>().unwrap_or(0)),
        None => (shortest, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let fraction = fraction.trim_end_matches('0');
    let Ok(digits) = format!("{whole}{fraction}").parse::<i128>() else {
        return Number::Float(n);
    };
    let Ok(zeros) = u32::try_from(exponent - fraction.len() as i32) else {
        return Number::Float(n);
    };
    let magnitude = digits * 10i128.pow(zeros);
    let value = if n < 0.0 { -magnitude } else { magnitude };
    if let Ok(u) = u64::try_from(value) {
        Number::Unsigned(u)
    } else if let Ok(i) = i64::try_from(value) {
        Number::Signed(i)
    } else {
        Number::Float(n)
    }
}

/// A macro-generated `&str` parameter: the string read in place for the length of one call.
#[doc(hidden)]
pub struct Str {
    borrowed: (*const u8, usize),
    owned: Option<String>,
}

pub(crate) const STR_TOKEN: &str = "\0skathi::Str";
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

impl Str {
    /// # Safety
    /// Only inside the call whose arguments produced it.
    pub unsafe fn get(&self) -> &str {
        match &self.owned {
            Some(s) => s,
            None => unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    self.borrowed.0,
                    self.borrowed.1,
                ))
            },
        }
    }
}

impl<'de> de::Deserialize<'de> for Str {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrVisitor;
        impl<'de> Visitor<'de> for StrVisitor {
            type Value = Str;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }
            fn visit_newtype_struct<D: de::Deserializer<'de>>(self, d: D) -> Result<Str, D::Error> {
                d.deserialize_str(self)
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Str, E> {
                Ok(Str {
                    borrowed: (v.as_ptr(), v.len()),
                    owned: None,
                })
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Str, E> {
                self.visit_string(v.to_owned())
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Str, E> {
                Ok(Str {
                    borrowed: (std::ptr::null(), 0),
                    owned: Some(v),
                })
            }
        }
        deserializer.deserialize_newtype_struct(STR_TOKEN, StrVisitor)
    }
}

#[cfg(feature = "types")]
impl specta::Type for Str {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        <String as specta::Type>::definition(types)
    }
}

/// serde_json gives up past this many nested arrays and objects, the arguments' array included.
const DEPTH: usize = 127;

/// The walk of one argument: the containers it is inside, for cycles and depth.
pub(crate) struct Walk<'a> {
    context: &'a Context,
    stack: RefCell<Vec<u64>>,
}

impl<'a> Walk<'a> {
    pub(crate) fn new(context: &'a Context) -> Self {
        Self {
            context,
            stack: RefCell::new(Vec::new()),
        }
    }

    /// One argument, read as JSON.stringify writes the `index`th element of the arguments.
    pub(crate) fn arg(&self, index: u32, value: Value) -> Js<'_, 'a> {
        Js {
            walk: self,
            key: Key::Index(index),
            value,
        }
    }

    fn enter(&self, value: Value) -> Result<Entered<'_, 'a>, Error> {
        let mut stack = self.stack.borrow_mut();
        if stack.contains(&value.0) {
            drop(stack);
            return Err(stringify_error(self.context, value));
        }
        if stack.len() + 1 >= DEPTH {
            return Err(not_json("recursion limit exceeded"));
        }
        stack.push(value.0);
        Ok(Entered { walk: self })
    }
}

struct Entered<'w, 'a> {
    walk: &'w Walk<'a>,
}

impl Drop for Entered<'_, '_> {
    fn drop(&mut self) {
        self.walk.stack.borrow_mut().pop();
    }
}

/// The key `toJSON` is called with.
#[derive(Clone, Copy)]
enum Key {
    Index(u32),
    Name(Value),
}

/// A value as JSON.stringify sees it, after `toJSON` and unboxing.
enum Node<'s> {
    Null,
    Bool(bool),
    Number(f64),
    Str(Cow<'s, str>),
    Array(Value),
    Object(Value),
}

/// One value of an argument, read the way `JSON.stringify` then serde_json would.
pub(crate) struct Js<'w, 'a> {
    walk: &'w Walk<'a>,
    key: Key,
    value: Value,
}

impl<'w, 'a> Js<'w, 'a> {
    fn context(&self) -> &'a Context {
        self.walk.context
    }

    /// SerializeJSONProperty: `None` is what an object leaves out and an array writes as null.
    fn node<'s>(&self) -> Result<Option<Node<'s>>, Error> {
        let context = self.context();
        let mut v = self.value;
        if let Some(node) = scalar(v) {
            return Ok(node);
        }
        // SAFETY: `v` is an argument or a property value held by this frame.
        if let Some(s) = unsafe { read_str(context, v) } {
            return s.map(|s| Some(Node::Str(s)));
        }
        let mut kind = context.kind(v);
        if kind == 6 {
            return Ok(None);
        }
        let to_json = with_atom(context, "toJSON", |atom| context.get(v, atom)).map_err(engine)?;
        if context.kind(to_json) == 9 {
            let key = match self.key {
                Key::Index(i) => string(context, &i.to_string()),
                Key::Name(name) => name,
            };
            v = context.call(to_json, v, &[key]).map_err(engine)?;
            if let Some(node) = scalar(v) {
                return Ok(node);
            }
            // SAFETY: as above; `v` stays on this frame.
            if let Some(s) = unsafe { read_str(context, v) } {
                return s.map(|s| Some(Node::Str(s)));
            }
            kind = context.kind(v);
            if kind == 6 {
                return Ok(None);
            }
        }
        match kind {
            7 => Err(stringify_error(context, v)),
            9 => Ok(None),
            10 => Ok(Some(Node::Array(v))),
            8 => self.object(v),
            _ => Ok(Some(Node::Object(v))),
        }
    }

    /// An object that may be a boxed primitive or an array behind a Proxy.
    fn object<'s>(&self, v: Value) -> Result<Option<Node<'s>>, Error> {
        let context = self.context();
        let mut inner = 0u64;
        // SAFETY: plain FFI on a live value.
        if unsafe { glue::rjs_wrapper_value(context.raw(), v.0, &mut inner) } == glue::OK {
            let inner = Value(inner);
            if let Some(b) = inner.as_bool() {
                return Ok(Some(Node::Bool(b)));
            }
            if inner.is_number() {
                let n = context.to_number(v).map_err(engine)?;
                return Ok(Some(if n.is_finite() {
                    Node::Number(n)
                } else {
                    Node::Null
                }));
            }
            match context.kind(inner) {
                5 => {
                    let s = context.to_rust_string(v).map_err(engine)?;
                    return Ok(Some(Node::Str(Cow::Owned(s))));
                }
                7 => return Err(stringify_error(context, inner)),
                _ => {}
            }
        }
        let is_array = context
            .call(intrinsics()?.is_array, Value::undefined(), &[v])
            .map_err(engine)?;
        Ok(Some(if is_array.as_bool() == Some(true) {
            Node::Array(v)
        } else {
            Node::Object(v)
        }))
    }

    fn require(&self) -> Result<Node<'_>, Error> {
        Ok(self.node()?.unwrap_or(Node::Null))
    }

    fn json(&self) -> Result<Json, Error> {
        through_json(self.context(), self.value)
    }

    fn child(&self, key: Key, value: Value) -> Js<'w, 'a> {
        Js {
            walk: self.walk,
            key,
            value,
        }
    }

    fn seq<'de, V: Visitor<'de>>(&self, array: Value, visitor: V) -> Result<V::Value, Error> {
        let _entered = self.walk.enter(array)?;
        let len = self.context().array_length(array).map_err(engine)?;
        let mut seq = Seq {
            parent: self,
            array,
            next: 0,
            len,
        };
        let value = visitor.visit_seq(&mut seq)?;
        if seq.next < len {
            return Err(de::Error::invalid_length(
                len as usize,
                &"fewer elements in array",
            ));
        }
        Ok(value)
    }

    fn map<'de, V: Visitor<'de>>(&self, object: Value, visitor: V) -> Result<V::Value, Error> {
        let _entered = self.walk.enter(object)?;
        let mut map = Map::new(self, object)?;
        let value = visitor.visit_map(&mut map)?;
        if map.next < map.len {
            let seen = map.taken;
            let rest = map.count_rest()?;
            return Err(de::Error::invalid_length(
                seen + rest,
                &"fewer elements in map",
            ));
        }
        Ok(value)
    }
}

fn scalar<'s>(v: Value) -> Option<Option<Node<'s>>> {
    if v.is_undefined() {
        return Some(None);
    }
    if v.is_null() {
        return Some(Some(Node::Null));
    }
    if let Some(b) = v.as_bool() {
        return Some(Some(Node::Bool(b)));
    }
    let n = v.as_number()?;
    Some(Some(if n.is_finite() {
        Node::Number(n)
    } else {
        Node::Null
    }))
}

fn unexpected<'s>(node: &'s Node<'_>) -> Unexpected<'s> {
    match node {
        Node::Null => Unexpected::Unit,
        Node::Bool(b) => Unexpected::Bool(*b),
        Node::Number(n) => number_unexpected(*n),
        Node::Str(s) => Unexpected::Str(s),
        Node::Array(_) => Unexpected::Seq,
        Node::Object(_) => Unexpected::Map,
    }
}

struct Seq<'p, 'w, 'a> {
    parent: &'p Js<'w, 'a>,
    array: Value,
    next: u32,
    len: u32,
}

impl<'de> de::SeqAccess<'de> for Seq<'_, '_, '_> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        if self.next >= self.len {
            return Ok(None);
        }
        let i = self.next;
        self.next += 1;
        let value = self
            .parent
            .context()
            .get_index(self.array, i)
            .map_err(engine)?;
        seed.deserialize(self.parent.child(Key::Index(i), value))
            .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some((self.len - self.next) as usize)
    }
}

/// An object's enumerable own string keys, in order, leaving out what JSON.stringify leaves out.
struct Map<'p, 'w, 'a> {
    parent: &'p Js<'w, 'a>,
    object: Value,
    keys: Value,
    next: u32,
    len: u32,
    taken: usize,
    value: Option<Value>,
    key: Value,
}

impl<'p, 'w, 'a> Map<'p, 'w, 'a> {
    fn new(parent: &'p Js<'w, 'a>, object: Value) -> Result<Self, Error> {
        let context = parent.context();
        let mut keys = 0u64;
        // SAFETY: plain FFI on a live object.
        match unsafe { glue::rjs_own_keys(context.raw(), object.0, &mut keys) } {
            glue::OK => {}
            glue::EXCEPTION => return Err(Error::Thrown(Value(keys))),
            _ => return Err(Error::Message("not an object".into())),
        }
        let keys = Value(keys);
        let len = context.array_length(keys).map_err(engine)?;
        Ok(Self {
            parent,
            object,
            keys,
            next: 0,
            len,
            taken: 0,
            value: None,
            key: Value::undefined(),
        })
    }

    /// The next key whose value JSON.stringify writes.
    fn advance(&mut self) -> Result<bool, Error> {
        let context = self.parent.context();
        while self.next < self.len {
            let key = context.get_index(self.keys, self.next).map_err(engine)?;
            self.next += 1;
            let mut out = 0u64;
            // SAFETY: plain FFI on live values.
            match unsafe { glue::rjs_get_v(context.raw(), self.object.0, key.0, &mut out) } {
                glue::OK => {}
                glue::EXCEPTION => return Err(Error::Thrown(Value(out))),
                _ => return Err(Error::Message("not an object".into())),
            }
            let value = Value(out);
            if self.parent.child(Key::Name(key), value).node()?.is_none() {
                continue;
            }
            self.key = key;
            self.value = Some(value);
            return Ok(true);
        }
        Ok(false)
    }

    fn count_rest(&mut self) -> Result<usize, Error> {
        let mut n = 0;
        while self.advance()? {
            n += 1;
        }
        Ok(n)
    }
}

impl<'de> de::MapAccess<'de> for Map<'_, '_, '_> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        if !self.advance()? {
            return Ok(None);
        }
        self.taken += 1;
        let context = self.parent.context();
        // SAFETY: the key string is held by `self.keys` for the whole walk.
        let key = match unsafe { read_str(context, self.key) } {
            Some(key) => key?,
            None => return Err(Error::Message("a key is not a string".into())),
        };
        seed.deserialize(KeyDe { key }).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        match self.value.take() {
            Some(value) => seed.deserialize(self.parent.child(Key::Name(self.key), value)),
            None => Err(de::Error::custom("value is missing")),
        }
    }
}

/// A JS object key, read as serde_json reads an object key.
struct KeyDe<'s> {
    key: Cow<'s, str>,
}

/// The length of the JSON number at the start of `s`.
fn number_prefix(s: &[u8]) -> usize {
    let mut i = usize::from(s.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let start = *i;
        while s.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i > start
    };
    if s.get(i) == Some(&b'0') {
        i += 1;
    } else if !digits(&mut i) {
        return i;
    }
    if s.get(i) == Some(&b'.') {
        let mut j = i + 1;
        if digits(&mut j) {
            i = j;
        }
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if digits(&mut j) {
            i = j;
        }
    }
    i
}

macro_rules! numeric_key {
    ($($method:ident)*) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            let expected = || Error::Message("invalid value: expected key to be a number in quotes".into());
            if !matches!(self.key.as_bytes().first(), Some(b'0'..=b'9' | b'-')) {
                return Err(expected());
            }
            let mut de = serde_json::Deserializer::from_str(&self.key);
            let value = de::Deserializer::$method(&mut de, Numbers(visitor, std::marker::PhantomData))
                .map_err(|e| Error::Message(e.to_string()))?;
            if number_prefix(self.key.as_bytes()) < self.key.len() {
                return Err(expected());
            }
            Ok(value)
        })*
    };
}

/// Hands a visitor the numbers serde_json reads out of a key, whose text does not live as long
/// as the visitor's input.
struct Numbers<'de, V>(V, std::marker::PhantomData<&'de ()>);

macro_rules! forward_numbers {
    ($($method:ident($ty:ty))*) => {
        $(fn $method<E: de::Error>(self, v: $ty) -> Result<Self::Value, E> {
            self.0.$method(v)
        })*
    };
}

impl<'x, 'de, V: Visitor<'de>> Visitor<'x> for Numbers<'de, V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }

    forward_numbers! {
        visit_i64(i64) visit_u64(u64) visit_i128(i128) visit_u128(u128) visit_f64(f64)
    }
}

impl<'de> de::Deserializer<'de> for KeyDe<'_> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.key {
            Cow::Borrowed(s) => visitor.visit_str(s),
            Cow::Owned(s) => visitor.visit_string(s),
        }
    }

    numeric_key! {
        deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64 deserialize_i128
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64 deserialize_u128
        deserialize_f32 deserialize_f64
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match &*self.key {
            "true" => visitor.visit_bool(true),
            "false" => visitor.visit_bool(false),
            other => Err(de::Error::invalid_type(Unexpected::Str(other), &visitor)),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_enum(
            IntoDeserializer::<Error>::into_deserializer(self.key.into_owned()),
            name,
            variants,
            visitor,
        )
    }

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf unit unit_struct seq tuple tuple_struct
        map struct identifier ignored_any
    }
}

macro_rules! numbers {
    ($($method:ident)*) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self.require()? {
                Node::Number(n) => visit_number(n, visitor),
                node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
            }
        })*
    };
}

macro_rules! strings {
    ($($method:ident)*) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self.require()? {
                Node::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
                Node::Str(Cow::Owned(s)) => visitor.visit_string(s),
                node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
            }
        })*
    };
}

impl<'de> de::Deserializer<'de> for Js<'_, '_> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Null => visitor.visit_unit(),
            Node::Bool(b) => visitor.visit_bool(b),
            Node::Number(n) => visit_number(n, visitor),
            Node::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
            Node::Str(Cow::Owned(s)) => visitor.visit_string(s),
            Node::Array(a) => self.seq(a, visitor),
            Node::Object(o) => self.map(o, visitor),
        }
    }

    numbers! {
        deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64 deserialize_i128
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64 deserialize_u128
        deserialize_f32 deserialize_f64
    }

    strings! {
        deserialize_char deserialize_str deserialize_string deserialize_identifier
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Bool(b) => visitor.visit_bool(b),
            node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_byte_buf(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
            Node::Str(Cow::Owned(s)) => visitor.visit_string(s),
            Node::Array(a) => self.seq(a, visitor),
            node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.node()? {
            None | Some(Node::Null) => visitor.visit_none(),
            Some(_) => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Null => visitor.visit_unit(),
            node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        if name == STR_TOKEN {
            if let Node::Str(Cow::Borrowed(s)) = self.require()? {
                // SAFETY: the engine's string outlives the call, which is as long as a `Str` is
                // used.
                let s: &'de str = unsafe { &*(s as *const str) };
                return visitor.visit_borrowed_str(s);
            }
            return self.deserialize_string(visitor);
        }
        if name == RAW_VALUE_TOKEN {
            return de::Deserializer::deserialize_newtype_struct(self.json()?, name, visitor)
                .map_err(|e| Error::Message(e.to_string()));
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Array(a) => self.seq(a, visitor),
            node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Object(o) => self.map(o, visitor),
            node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Array(a) => self.seq(a, visitor),
            Node::Object(o) => self.map(o, visitor),
            node => Err(de::Error::invalid_type(unexpected(&node), &visitor)),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.require()? {
            Node::Str(s) => {
                visitor.visit_enum(IntoDeserializer::<Error>::into_deserializer(s.into_owned()))
            }
            Node::Object(o) => {
                let _entered = self.walk.enter(o)?;
                let mut map = Map::new(&self, o)?;
                let single = || de::Error::invalid_value(Unexpected::Map, &"map with a single key");
                if !map.advance()? {
                    return Err(single());
                }
                let (key, value) = (map.key, map.value.take());
                if map.advance()? {
                    return Err(single());
                }
                let context = self.context();
                // SAFETY: the key string is held by `map.keys` for the whole walk.
                let variant = match unsafe { read_str(context, key) } {
                    Some(s) => s?.into_owned(),
                    None => return Err(de::Error::custom("a key is not a string")),
                };
                let value = value.map(|v| self.child(Key::Name(key), v));
                visitor.visit_enum(Variant { variant, value })
            }
            node => Err(de::Error::invalid_type(unexpected(&node), &"string or map")),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_any(de::IgnoredAny)?;
        visitor.visit_unit()
    }
}

/// `{ "Variant": value }`, the externally tagged form.
struct Variant<'w, 'a> {
    variant: String,
    value: Option<Js<'w, 'a>>,
}

impl<'de, 'w, 'a> de::EnumAccess<'de> for Variant<'w, 'a> {
    type Error = Error;
    type Variant = VariantValue<'w, 'a>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Error> {
        let variant =
            seed.deserialize(IntoDeserializer::<Error>::into_deserializer(self.variant))?;
        Ok((variant, VariantValue(self.value)))
    }
}

struct VariantValue<'w, 'a>(Option<Js<'w, 'a>>);

impl<'de> de::VariantAccess<'de> for VariantValue<'_, '_> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        match self.0 {
            Some(value) => de::Deserialize::deserialize(value),
            None => Ok(()),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        match self.0 {
            Some(value) => seed.deserialize(value),
            None => Err(de::Error::invalid_type(
                Unexpected::UnitVariant,
                &"newtype variant",
            )),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Error> {
        let Some(value) = self.0 else {
            return Err(de::Error::invalid_type(
                Unexpected::UnitVariant,
                &"tuple variant",
            ));
        };
        match value.require()? {
            Node::Array(a) => {
                let len = value.context().array_length(a).map_err(engine)?;
                if len == 0 {
                    let _entered = value.walk.enter(a)?;
                    visitor.visit_unit()
                } else {
                    value.seq(a, visitor)
                }
            }
            node => Err(de::Error::invalid_type(unexpected(&node), &"tuple variant")),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let Some(value) = self.0 else {
            return Err(de::Error::invalid_type(
                Unexpected::UnitVariant,
                &"struct variant",
            ));
        };
        match value.require()? {
            Node::Object(o) => value.map(o, visitor),
            node => Err(de::Error::invalid_type(
                unexpected(&node),
                &"struct variant",
            )),
        }
    }
}

/// How JSON.parse reads what serde_json writes for an `f32`: its shortest decimal form.
pub(crate) fn f32_number(v: f32) -> f64 {
    zmij::Buffer::new()
        .format_finite(v)
        .parse()
        .unwrap_or(v as f64)
}

/// Where a result is built: engine values on the JS thread, or an [`Owned`] tree anywhere.
pub(crate) trait Build: Copy {
    type Value;
    type Array;
    type Object;
    fn null(self) -> Self::Value;
    fn bool(self, v: bool) -> Self::Value;
    fn number(self, v: f64) -> Self::Value;
    fn string(self, v: &str) -> Self::Value;
    fn raw_json(self, json: &str) -> Result<Self::Value, Error>;
    fn array(self, len: usize) -> Result<Self::Array, Error>;
    fn push(self, array: &mut Self::Array, v: Self::Value) -> Result<(), Error>;
    fn end_array(self, array: Self::Array) -> Self::Value;
    fn object(self) -> Result<Self::Object, Error>;
    fn field(
        self,
        object: &mut Self::Object,
        key: &'static str,
        v: Self::Value,
    ) -> Result<(), Error>;
    fn entry(self, object: &mut Self::Object, key: String, v: Self::Value) -> Result<(), Error>;
    fn end_object(self, object: Self::Object) -> Self::Value;
}

/// Whether CreateDataProperty treats `key` as an array index.
fn is_index(key: &str) -> bool {
    let b = key.as_bytes();
    !b.is_empty()
        && b.len() <= 10
        && b.iter().all(u8::is_ascii_digit)
        && (b[0] != b'0' || b.len() == 1)
        && key.parse::<u64>().is_ok_and(|n| n < u32::MAX as u64)
}

#[derive(Clone, Copy)]
pub(crate) struct Engine<'a>(pub(crate) &'a Context);

impl<'a> Engine<'a> {
    fn define(self, object: Value, key: Value, v: Value) -> Result<(), Error> {
        // SAFETY: plain FFI on live values.
        match unsafe { glue::rjs_define_v(self.0.raw(), object.0, key.0, v.0) } {
            glue::OK => Ok(()),
            _ => Err(Error::Message("the result cannot be built".into())),
        }
    }
}

impl<'a> Build for Engine<'a> {
    type Value = Value;
    type Array = (Value, u32);
    type Object = Value;

    fn null(self) -> Value {
        Value::null()
    }
    fn bool(self, v: bool) -> Value {
        Value::boolean(v)
    }
    fn number(self, v: f64) -> Value {
        if v.is_finite() {
            Value::number(v)
        } else {
            Value::null()
        }
    }
    fn string(self, v: &str) -> Value {
        string(self.0, v)
    }
    fn raw_json(self, json: &str) -> Result<Value, Error> {
        parse(self.0, json)
    }
    fn array(self, len: usize) -> Result<(Value, u32), Error> {
        let array = self
            .0
            .array(u32::try_from(len).unwrap_or(0))
            .map_err(engine)?;
        Ok((array, 0))
    }
    fn push(self, array: &mut (Value, u32), v: Value) -> Result<(), Error> {
        // SAFETY: plain FFI on live values.
        match unsafe { glue::rjs_define_index(self.0.raw(), array.0 .0, array.1, v.0) } {
            glue::OK => {
                array.1 += 1;
                Ok(())
            }
            _ => Err(Error::Message("the result cannot be built".into())),
        }
    }
    fn end_array(self, array: (Value, u32)) -> Value {
        array.0
    }
    fn object(self) -> Result<Value, Error> {
        Ok(self.0.object())
    }
    fn field(self, object: &mut Value, key: &'static str, v: Value) -> Result<(), Error> {
        if is_index(key) {
            return self.define(*object, string(self.0, key), v);
        }
        with_atom(self.0, key, |atom| self.0.define(*object, atom, v, 0)).map_err(engine)
    }
    fn entry(self, object: &mut Value, key: String, v: Value) -> Result<(), Error> {
        let key = string(self.0, &key);
        self.define(*object, key, v)
    }
    fn end_object(self, object: Value) -> Value {
        object
    }
}

/// A result or an event made off the JS thread, built into engine values on it.
#[derive(Debug, Clone, PartialEq)]
#[doc(hidden)]
pub enum Owned {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Owned>),
    Object(Vec<(Cow<'static, str>, Owned)>),
    Json(String),
}

#[derive(Clone, Copy)]
pub(crate) struct Tree;

impl Build for Tree {
    type Value = Owned;
    type Array = Vec<Owned>;
    type Object = Vec<(Cow<'static, str>, Owned)>;

    fn null(self) -> Owned {
        Owned::Null
    }
    fn bool(self, v: bool) -> Owned {
        Owned::Bool(v)
    }
    fn number(self, v: f64) -> Owned {
        if v.is_finite() {
            Owned::Number(v)
        } else {
            Owned::Null
        }
    }
    fn string(self, v: &str) -> Owned {
        Owned::Str(v.to_owned())
    }
    fn raw_json(self, json: &str) -> Result<Owned, Error> {
        Ok(Owned::Json(json.to_owned()))
    }
    fn array(self, len: usize) -> Result<Vec<Owned>, Error> {
        Ok(Vec::with_capacity(len))
    }
    fn push(self, array: &mut Vec<Owned>, v: Owned) -> Result<(), Error> {
        array.push(v);
        Ok(())
    }
    fn end_array(self, array: Vec<Owned>) -> Owned {
        Owned::Array(array)
    }
    fn object(self) -> Result<Self::Object, Error> {
        Ok(Vec::new())
    }
    fn field(self, object: &mut Self::Object, key: &'static str, v: Owned) -> Result<(), Error> {
        object.push((Cow::Borrowed(key), v));
        Ok(())
    }
    fn entry(self, object: &mut Self::Object, key: String, v: Owned) -> Result<(), Error> {
        object.push((Cow::Owned(key), v));
        Ok(())
    }
    fn end_object(self, object: Self::Object) -> Owned {
        Owned::Object(object)
    }
}

impl Owned {
    pub(crate) fn to_js(&self, context: &Context) -> Result<Value, Error> {
        let b = Engine(context);
        Ok(match self {
            Owned::Null => Value::null(),
            Owned::Bool(v) => Value::boolean(*v),
            Owned::Number(v) => b.number(*v),
            Owned::Str(s) => string(context, s),
            Owned::Json(json) => parse(context, json)?,
            Owned::Array(items) => {
                let mut array = b.array(items.len())?;
                for item in items {
                    let v = item.to_js(context)?;
                    b.push(&mut array, v)?;
                }
                array.0
            }
            Owned::Object(fields) => {
                let mut object = context.object();
                for (key, item) in fields {
                    let v = item.to_js(context)?;
                    match key {
                        Cow::Borrowed(key) => b.field(&mut object, key, v)?,
                        Cow::Owned(key) => b.entry(&mut object, key.clone(), v)?,
                    }
                }
                object
            }
        })
    }
}

#[derive(Debug)]
pub(crate) enum OutError {
    Message(String),
    /// The top-level `Err(e)`, as serde_json writes `e`.
    Err(String),
    Thrown(Value),
}

impl fmt::Display for OutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutError::Message(m) | OutError::Err(m) => f.write_str(m),
            OutError::Thrown(_) => f.write_str("a JS exception"),
        }
    }
}

impl std::error::Error for OutError {}

impl ser::Error for OutError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        OutError::Message(msg.to_string())
    }
}

impl From<Error> for OutError {
    fn from(e: Error) -> Self {
        match e {
            Error::Thrown(v) => OutError::Thrown(v),
            Error::Message(m) | Error::NotJson(m) => OutError::Message(m),
        }
    }
}

/// Builds a serde value as serde_json writes it and JSON.parse reads it back. `top`: a
/// `Result` here is the call's outcome, not a value.
pub(crate) struct Out<B> {
    pub(crate) build: B,
    pub(crate) top: bool,
}

impl<B: Build> Out<B> {
    fn inner(&self) -> Out<B> {
        Out {
            build: self.build,
            top: false,
        }
    }
}

pub(crate) struct Compound<B: Build> {
    build: B,
    array: Option<B::Array>,
    object: Option<B::Object>,
    variant: Option<&'static str>,
    key: Option<String>,
    raw: bool,
}

impl<B: Build> Compound<B> {
    fn array(build: B, len: usize, variant: Option<&'static str>) -> Result<Self, OutError> {
        Ok(Self {
            build,
            array: Some(build.array(len)?),
            object: None,
            variant,
            key: None,
            raw: false,
        })
    }

    fn object(build: B, variant: Option<&'static str>) -> Result<Self, OutError> {
        Ok(Self {
            build,
            array: None,
            object: Some(build.object()?),
            variant,
            key: None,
            raw: false,
        })
    }

    fn element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        let v = v.serialize(Out {
            build: self.build,
            top: false,
        })?;
        match &mut self.array {
            Some(array) => Ok(self.build.push(array, v)?),
            None => Err(OutError::Message("not an array".into())),
        }
    }

    fn field<T: Serialize + ?Sized>(&mut self, key: &'static str, v: &T) -> Result<(), OutError> {
        let v = v.serialize(Out {
            build: self.build,
            top: false,
        })?;
        match &mut self.object {
            Some(object) => Ok(self.build.field(object, key, v)?),
            None => Err(OutError::Message("not an object".into())),
        }
    }

    fn finish(self) -> Result<B::Value, OutError> {
        let value = match (self.array, self.object) {
            (Some(array), _) => self.build.end_array(array),
            (_, Some(object)) => self.build.end_object(object),
            _ => return Err(OutError::Message("an empty compound".into())),
        };
        match self.variant {
            Some(variant) => {
                let mut outer = self.build.object()?;
                self.build.field(&mut outer, variant, value)?;
                Ok(self.build.end_object(outer))
            }
            None => Ok(value),
        }
    }
}

impl<B: Build> ser::SerializeSeq for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        self.element(v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        self.finish()
    }
}

impl<B: Build> ser::SerializeTuple for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        self.element(v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        self.finish()
    }
}

impl<B: Build> ser::SerializeTupleStruct for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        self.element(v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        self.finish()
    }
}

impl<B: Build> ser::SerializeTupleVariant for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        self.element(v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        self.finish()
    }
}

impl<B: Build> ser::SerializeMap for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), OutError> {
        self.key = Some(key.serialize(KeySer)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        let key = self
            .key
            .take()
            .ok_or_else(|| OutError::Message("a map value without a key".into()))?;
        let v = v.serialize(Out {
            build: self.build,
            top: false,
        })?;
        match &mut self.object {
            Some(object) => Ok(self.build.entry(object, key, v)?),
            None => Err(OutError::Message("not an object".into())),
        }
    }
    fn end(self) -> Result<B::Value, OutError> {
        self.finish()
    }
}

impl<B: Build> ser::SerializeStruct for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), OutError> {
        if self.raw {
            let Owned::Str(json) = v.serialize(Out {
                build: Tree,
                top: false,
            })?
            else {
                return Err(OutError::Message("a RawValue without its JSON".into()));
            };
            self.key = Some(json);
            return Ok(());
        }
        self.field(key, v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        if self.raw {
            let json = self.key.unwrap_or_default();
            return Ok(self.build.raw_json(&json)?);
        }
        self.finish()
    }
}

impl<B: Build> ser::SerializeStructVariant for Compound<B> {
    type Ok = B::Value;
    type Error = OutError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), OutError> {
        self.field(key, v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        self.finish()
    }
}

impl<B: Build> ser::Serializer for Out<B> {
    type Ok = B::Value;
    type Error = OutError;
    type SerializeSeq = Compound<B>;
    type SerializeTuple = Compound<B>;
    type SerializeTupleStruct = Compound<B>;
    type SerializeTupleVariant = Compound<B>;
    type SerializeMap = Compound<B>;
    type SerializeStruct = Compound<B>;
    type SerializeStructVariant = Compound<B>;

    fn serialize_bool(self, v: bool) -> Result<B::Value, OutError> {
        Ok(self.build.bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_i16(self, v: i16) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_i32(self, v: i32) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_i64(self, v: i64) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_i128(self, v: i128) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_u8(self, v: u8) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_u16(self, v: u16) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_u32(self, v: u32) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_u64(self, v: u64) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_u128(self, v: u128) -> Result<B::Value, OutError> {
        Ok(self.build.number(v as f64))
    }
    fn serialize_f32(self, v: f32) -> Result<B::Value, OutError> {
        Ok(if v.is_finite() {
            self.build.number(f32_number(v))
        } else {
            self.build.null()
        })
    }
    fn serialize_f64(self, v: f64) -> Result<B::Value, OutError> {
        Ok(self.build.number(v))
    }
    fn serialize_char(self, v: char) -> Result<B::Value, OutError> {
        Ok(self.build.string(v.encode_utf8(&mut [0; 4])))
    }
    fn serialize_str(self, v: &str) -> Result<B::Value, OutError> {
        Ok(self.build.string(v))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<B::Value, OutError> {
        let mut array = self.build.array(v.len())?;
        for &byte in v {
            self.build
                .push(&mut array, self.build.number(byte as f64))?;
        }
        Ok(self.build.end_array(array))
    }
    fn serialize_none(self) -> Result<B::Value, OutError> {
        Ok(self.build.null())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<B::Value, OutError> {
        v.serialize(self.inner())
    }
    fn serialize_unit(self) -> Result<B::Value, OutError> {
        Ok(self.build.null())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<B::Value, OutError> {
        Ok(self.build.null())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<B::Value, OutError> {
        Ok(self.build.string(variant))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<B::Value, OutError> {
        v.serialize(self.inner())
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<B::Value, OutError> {
        if self.top && name == "Result" {
            match variant {
                "Ok" => return v.serialize(self.inner()),
                "Err" => {
                    return Err(match serde_json::to_string(v) {
                        Ok(json) => OutError::Err(json),
                        Err(e) => OutError::Message(e.to_string()),
                    })
                }
                _ => {}
            }
        }
        let inner = v.serialize(self.inner())?;
        let mut object = self.build.object()?;
        self.build.field(&mut object, variant, inner)?;
        Ok(self.build.end_object(object))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Compound<B>, OutError> {
        Compound::array(self.build, len.unwrap_or(0), None)
    }
    fn serialize_tuple(self, len: usize) -> Result<Compound<B>, OutError> {
        Compound::array(self.build, len, None)
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Compound<B>, OutError> {
        Compound::array(self.build, len, None)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Compound<B>, OutError> {
        Compound::array(self.build, len, Some(variant))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Compound<B>, OutError> {
        Compound::object(self.build, None)
    }
    fn serialize_struct(self, name: &'static str, _: usize) -> Result<Compound<B>, OutError> {
        let mut compound = Compound::object(self.build, None)?;
        compound.raw = name == RAW_VALUE_TOKEN;
        Ok(compound)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<B>, OutError> {
        Compound::object(self.build, Some(variant))
    }
}

fn key_must_be_a_string() -> OutError {
    OutError::Message("key must be a string".into())
}

/// A map key as serde_json writes it: a string, or the text of a number or a bool.
struct KeySer;

impl ser::Serializer for KeySer {
    type Ok = String;
    type Error = OutError;
    type SerializeSeq = ser::Impossible<String, OutError>;
    type SerializeTuple = ser::Impossible<String, OutError>;
    type SerializeTupleStruct = ser::Impossible<String, OutError>;
    type SerializeTupleVariant = ser::Impossible<String, OutError>;
    type SerializeMap = ser::Impossible<String, OutError>;
    type SerializeStruct = ser::Impossible<String, OutError>;
    type SerializeStructVariant = ser::Impossible<String, OutError>;

    fn serialize_bool(self, v: bool) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_i8(self, v: i8) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_i128(self, v: i128) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_u128(self, v: u128) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_f32(self, v: f32) -> Result<String, OutError> {
        if !v.is_finite() {
            return Err(OutError::Message(
                "float key must be finite (got NaN or +/-inf)".into(),
            ));
        }
        Ok(zmij::Buffer::new().format_finite(v).to_owned())
    }
    fn serialize_f64(self, v: f64) -> Result<String, OutError> {
        if !v.is_finite() {
            return Err(OutError::Message(
                "float key must be finite (got NaN or +/-inf)".into(),
            ));
        }
        Ok(zmij::Buffer::new().format_finite(v).to_owned())
    }
    fn serialize_char(self, v: char) -> Result<String, OutError> {
        Ok(v.to_string())
    }
    fn serialize_str(self, v: &str) -> Result<String, OutError> {
        Ok(v.to_owned())
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<String, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_none(self) -> Result<String, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<String, OutError> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<String, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<String, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<String, OutError> {
        Ok(variant.to_owned())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<String, OutError> {
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<String, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, OutError> {
        Err(key_must_be_a_string())
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, OutError> {
        Err(key_must_be_a_string())
    }
}

#[cfg(test)]
mod tests {
    //! Conformance: every value of the corpus, read as every type, gives what the JSON round
    //! trip gave, value for value and error for error.

    use std::collections::BTreeMap;
    use std::fmt::Debug;

    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};
    use serde_json::value::RawValue;
    use sk_runtime::JsRuntime;

    use super::*;

    /// The arguments' way before: `JSON.stringify([value])`, serde_json, `from_value`.
    fn through_json_then<T: DeserializeOwned>(
        context: &Context,
        value: Value,
    ) -> Result<T, String> {
        let stringify = intrinsics().unwrap().stringify;
        let array = context.array(1).unwrap();
        context.set_index(array, 0, value).unwrap();
        let text = match context.call(stringify, Value::undefined(), &[array]) {
            Ok(text) => context.read_string(text).unwrap(),
            Err(crate::js::Error::Exception(e)) => return Err(thrown(context, e)),
            Err(e) => return Err(e.to_string()),
        };
        let mut list: Vec<Json> = serde_json::from_str(&text).map_err(|e| {
            let e = e.to_string();
            let e = e.split(" at line ").next().unwrap_or_default().to_owned();
            format!("TypeError: the arguments are not JSON: {e}")
        })?;
        serde_json::from_value(list.remove(0)).map_err(|e| format!("argument 1: {e}"))
    }

    fn direct<T: DeserializeOwned>(context: &Context, value: Value) -> Result<T, String> {
        let walk = Walk::new(context);
        T::deserialize(walk.arg(0, value)).map_err(|e| match e {
            Error::Message(m) => format!("argument 1: {m}"),
            Error::Thrown(e) => thrown(context, e),
            Error::NotJson(m) => format!("TypeError: {m}"),
        })
    }

    fn thrown(context: &Context, e: Value) -> String {
        let name = context
            .get(e, &context.atom("name"))
            .and_then(|n| context.to_rust_string(n))
            .unwrap_or_default();
        format!("{name}: {}", context.error_info(e).0)
    }

    const VALUES: &[&str] = &[
        "undefined",
        "null",
        "true",
        "false",
        "0",
        "-0",
        "1",
        "-1",
        "42",
        "1.5",
        "-2.25",
        "0.1",
        "1e-7",
        "123456789.125",
        "NaN",
        "Infinity",
        "-Infinity",
        "2 ** 31",
        "2 ** 31 - 1",
        "-(2 ** 31)",
        "2 ** 32",
        "2 ** 53",
        "2 ** 53 + 2",
        "2 ** 63",
        "2 ** 64",
        "2 ** 70",
        "1e21",
        "1e300",
        "0.1 + 0.2",
        "1 / 3",
        "Math.PI",
        "-Math.E * 1e10",
        "1e-300",
        "5e-324",
        "Number.MAX_VALUE",
        "2 ** -1074 * 3",
        "123456.789e3",
        "0.000001234",
        "Math.sqrt(2) * 1e15",
        "9007199254740993.5 / 3",
        "Array.from({ length: 4000 }, (_, i) => Math.sin(i + 1) * 10 ** ((i % 60) - 30))",
        "Array.from({ length: 2000 }, (_, i) => Math.round(Math.cos(i) * 2 ** (40 + (i % 40))))",
        "Array.from({ length: 300 }, (_, i) => Math.cos(i) * 10 ** (i - 150))",
        "-(2 ** 63)",
        "-(2 ** 63) - 4096",
        "-1e21",
        "255",
        "256",
        "-128",
        "-129",
        "65536",
        "''",
        "'x'",
        "'hello'",
        "'h\\u00e9llo'",
        "'\\u00df\\u4e2d\\ud83d\\ude00'",
        "'\\ud800'",
        "'\\ud800x'",
        "'a\\ud800\\n'",
        "'\\ud800\\u0001'",
        "'\\ud800\\ud800'",
        "'\\udc00'",
        "'true'",
        "'false'",
        "'1'",
        "'-5'",
        "'1.5'",
        "'Unit'",
        "'Wrap'",
        "'B'",
        "[]",
        "[1, 2]",
        "[1, 2, 3]",
        "[1, 'a']",
        "[1, undefined, null]",
        "[() => 1, Symbol()]",
        "[[1], [2, 3]]",
        "[1.5, NaN]",
        "new Array(2)",
        "['a', 'b']",
        "{}",
        "({ x: 1, y: 2 })",
        "({ y: 2, x: 1 })",
        "({ x: 1, y: 2, z: 3 })",
        "({ x: 1 })",
        "({ x: undefined, y: 2 })",
        "({ x: 1, y: 2, f() {} })",
        "({ x: 1, y: 2, s: Symbol() })",
        "({ '1': 'a', '2': 'b' })",
        "({ b: 1, '2': 'b', a: 2, '1': 'a' })",
        "({ a: 1 })",
        "({ Unit: null })",
        "({ Unit: 1 })",
        "({ Wrap: 5 })",
        "({ Wrap: 'x' })",
        "({ Pair: [1, 'a'] })",
        "({ Pair: [] })",
        "({ Pair: 1 })",
        "({ Named: { x: 1, y: 2 } })",
        "({ Named: [1, 2] })",
        "({ Wrap: 1, Pair: [1, 'a'] })",
        "({ tag: 'A', x: 1 })",
        "({ tag: 'B' })",
        "({ t: 'A', c: 3 })",
        "({ t: 'B' })",
        "({ true: 1, false: 2 })",
        "({ '-1': 1, '1.5': 2, x: 3 })",
        "({ '01': 1, ' 1': 2, '1 ': 3, '1e3': 4, '300': 5, '-0': 6 })",
        "({ some_field: 1, someField: 2 })",
        "({ x: 1, y: 2, extra: { deep: [1, { z: 10n }] } })",
        "({ x: 1, y: 2, extra: '\\ud800' })",
        "new Date(0)",
        "new Number(5)",
        "new String('s')",
        "new Boolean(false)",
        "Object(Symbol())",
        "Object(10n)",
        "Symbol()",
        "() => 1",
        "10n",
        "[10n]",
        "({ x: 10n })",
        "(() => { const o = { x: 1 }; o.self = o; return o; })()",
        "(() => { const a = [1]; a.push(a); return a; })()",
        "(() => { const o = { x: 1, y: 2 }; return [o, o]; })()",
        "({ toJSON() { return { x: 7, y: 8 }; } })",
        "({ toJSON(key) { return key; } })",
        "({ toJSON() { return undefined; } })",
        "[{ toJSON(key) { return key + '!'; } }]",
        "({ x: { toJSON(key) { return key.length; } }, y: 2 })",
        "({ get x() { return 3; }, y: 4 })",
        "({ get x() { throw new Error('getter'); }, y: 4 })",
        "({ toJSON() { throw new RangeError('toJSON'); } })",
        "new Proxy([1, 2], {})",
        "new Proxy({ x: 1, y: 2 }, {})",
        "new Proxy({ x: 1, y: 2 }, { get(t, k) { return k === 'x' ? 9 : t[k]; } })",
        "new Map([[1, 2]])",
        "new Set([1])",
        "new Uint8Array([1, 2])",
        "new ArrayBuffer(2)",
        "new DataView(new ArrayBuffer(2))",
        "new Float32Array([1.5])",
        "/re/g",
        "new Error('e')",
        "Object.create({ x: 1 })",
        "Object.defineProperty({ y: 2 }, 'x', { value: 1, enumerable: false })",
        "Object.assign(Object.create(null), { x: 1, y: 2 })",
        "(() => { class P { constructor() { this.x = 1; this.y = 2; } get z() { return 3; } } return new P(); })()",
        "(() => { let v = 0; for (let i = 0; i < 130; i++) v = [v]; return v; })()",
        "(() => { let v = 0; for (let i = 0; i < 120; i++) v = [v]; return v; })()",
        "(() => { let v = { x: 1 }; for (let i = 0; i < 130; i++) v = { x: 1, y: 2, n: v }; return v; })()",
    ];

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Defaults {
        x: Option<i32>,
        #[serde(default)]
        y: u8,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Camel {
        some_field: i32,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        x: i32,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Flat {
        x: i32,
        #[serde(flatten)]
        rest: BTreeMap<String, Json>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Nested {
        x: i32,
        y: i32,
        n: Option<Box<Nested>>,
    }

    #[derive(Debug, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize)]
    enum Shape {
        Unit,
        Wrap(i32),
        Pair(i32, String),
        Named { x: i32, y: i32 },
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "tag")]
    enum Tagged {
        A { x: i32 },
        B,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "t", content = "c")]
    enum Adjacent {
        A(i32),
        B,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(untagged)]
    enum Either {
        N(i32),
        S(String),
        P(Point),
        L(Vec<Either>),
    }

    #[derive(Debug, PartialEq, PartialOrd, Eq, Ord, Serialize, Deserialize)]
    enum Key {
        Unit,
        B,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Newtype(i32);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Pair(i32, i32);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Unit;

    #[derive(Debug, Deserialize)]
    struct Raw(Box<RawValue>);

    impl PartialEq for Raw {
        fn eq(&self, other: &Self) -> bool {
            self.0.get() == other.0.get()
        }
    }

    fn eval(context: &Context, src: &str) -> Value {
        context.eval(&format!("(() => ({src}))()")).unwrap()
    }

    fn arguments<T: DeserializeOwned + Debug + PartialEq>(
        context: &Context,
        name: &str,
        mismatches: &mut Vec<String>,
    ) {
        for src in VALUES {
            let before = through_json_then::<T>(context, eval(context, src));
            let now = direct::<T>(context, eval(context, src));
            // A value that is not JSON as a whole still fails, but reading stops at the first
            // problem, so a wrong type met before it is what gets reported.
            let whole = |e: &String| !e.starts_with("argument 1: ");
            let agree = match (&before, &now) {
                (Err(old), Err(_)) if whole(old) => true,
                _ => before == now,
            };
            if !agree {
                mismatches.push(format!(
                    "{name} from {src}:\n  JSON:   {before:?}\n  direct: {now:?}"
                ));
            }
        }
    }

    #[test]
    fn arguments_read_directly_mean_what_the_json_round_trip_meant() {
        let mismatches = sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                install(context).unwrap();
                let mut m = Vec::new();
                macro_rules! check {
                    ($($t:ty),* $(,)?) => {
                        $(arguments::<$t>(context, stringify!($t), &mut m);)*
                    };
                }
                check!(
                    bool, i8, i32, u8, u32, i64, u64, i128, u128, f32, f64, char, String, (),
                    Option<i32>, Option<String>, Option<Point>, Vec<i32>, Vec<f64>,
                    Vec<Option<i32>>, Vec<String>, (i32, String), [u8; 2], Vec<Vec<i32>>,
                    BTreeMap<String, i32>, BTreeMap<u32, String>, BTreeMap<i8, i32>,
                    BTreeMap<bool, i32>, BTreeMap<char, i32>, BTreeMap<Key, i32>,
                    BTreeMap<String, Json>, Point, Defaults, Camel, Strict, Flat, Nested, Shape,
                    Tagged, Adjacent, Either, Newtype, Pair, Unit, Json, Raw,
                    Option<Shape>, Vec<Shape>,
                );
                m
            })
        });
        assert!(
            mismatches.is_empty(),
            "{} differences:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    /// Serializes as serde_json writes with `f64` map keys, which no std map has.
    struct FloatKeys(&'static [(f64, i32)]);

    impl Serialize for FloatKeys {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = s.serialize_map(Some(self.0.len()))?;
            for (k, v) in self.0 {
                map.serialize_entry(k, v)?;
            }
            map.end()
        }
    }

    struct Keyed<K: 'static, V: 'static>(&'static [(K, V)]);

    impl<K: Serialize, V: Serialize> Serialize for Keyed<K, V> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = s.serialize_map(None)?;
            for (k, v) in self.0 {
                map.serialize_entry(k, v)?;
            }
            map.end()
        }
    }

    struct Fails;

    impl Serialize for Fails {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("no can do"))
        }
    }

    struct RawBytes(&'static [u8]);

    impl Serialize for RawBytes {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bytes(self.0)
        }
    }

    fn result<T: Serialize>(context: &Context, name: &str, value: T, mismatches: &mut Vec<String>) {
        let before = serde_json::to_string(&value)
            .map_err(|e| e.to_string())
            .map(|json| parse(context, &json).unwrap());
        let now = value
            .serialize(Out {
                build: Engine(context),
                top: false,
            })
            .map_err(|e| e.to_string());
        let owned = value
            .serialize(Out {
                build: Tree,
                top: false,
            })
            .map_err(|e| e.to_string())
            .map(|tree| tree.to_js(context).unwrap());
        let same = intrinsics_same(context);
        for (way, got) in [("direct", now), ("owned", owned)] {
            let equal = match (&before, &got) {
                (Ok(a), Ok(b)) => context
                    .call(same, Value::undefined(), &[*a, *b])
                    .unwrap()
                    .as_bool()
                    .unwrap(),
                (Err(a), Err(b)) => a == b,
                _ => false,
            };
            if !equal {
                let show = |r: &Result<Value, String>| match r {
                    Ok(v) => context
                        .to_rust_string(
                            context
                                .eval("(v) => JSON.stringify(v) + ' ' + Object.is(v, -0)")
                                .and_then(|f| context.call(f, Value::undefined(), &[*v]))
                                .unwrap(),
                        )
                        .unwrap(),
                    Err(e) => format!("Err({e})"),
                };
                mismatches.push(format!(
                    "{name} ({way}):\n  JSON:   {}\n  direct: {}",
                    show(&before),
                    show(&got)
                ));
            }
        }
    }

    fn intrinsics_same(context: &Context) -> Value {
        context
            .eval(
                "(() => {
                    const same = (a, b) => {
                        if (typeof a !== typeof b) return false;
                        if (typeof a !== 'object' || a === null || b === null) return Object.is(a, b);
                        if (Array.isArray(a) !== Array.isArray(b)) return false;
                        if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;
                        const ka = Reflect.ownKeys(a), kb = Reflect.ownKeys(b);
                        if (ka.length !== kb.length) return false;
                        for (let i = 0; i < ka.length; i++) {
                            if (ka[i] !== kb[i]) return false;
                            const da = Object.getOwnPropertyDescriptor(a, ka[i]);
                            const db = Object.getOwnPropertyDescriptor(b, kb[i]);
                            if (da.enumerable !== db.enumerable || da.writable !== db.writable) return false;
                            if (!same(da.value, db.value)) return false;
                        }
                        return true;
                    };
                    return same;
                })()",
            )
            .unwrap()
    }

    #[test]
    fn results_built_directly_are_what_json_parse_made_of_serde_json() {
        let mismatches = sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                install(context).unwrap();
                let mut m = Vec::new();
                macro_rules! check {
                    ($($v:expr),* $(,)?) => {
                        $(result(context, stringify!($v), $v, &mut m);)*
                    };
                }
                check!(
                    true,
                    0i32,
                    -7i8,
                    u8::MAX,
                    u32::MAX,
                    i64::MIN,
                    u64::MAX,
                    i128::MIN,
                    u128::MAX,
                    1.5f64,
                    -0.0f64,
                    0.1f64,
                    1e21f64,
                    f64::NAN,
                    f64::INFINITY,
                    0.1f32,
                    1.0e-7f32,
                    3.4028235e38f32,
                    -0.0f32,
                    f32::NAN,
                    'x',
                    "h\u{e9}llo \u{1f600}",
                    "",
                    (),
                    None::<i32>,
                    Some(3),
                    Some(None::<i32>),
                    vec![1, 2, 3],
                    Vec::<i32>::new(),
                    (1, "a"),
                    [0u8; 3],
                    Point { x: 1, y: -2 },
                    Defaults { x: None, y: 3 },
                    Camel { some_field: 4 },
                    Nested {
                        x: 1,
                        y: 2,
                        n: Some(Box::new(Nested {
                            x: 3,
                            y: 4,
                            n: None
                        }))
                    },
                    Shape::Unit,
                    Shape::Wrap(5),
                    Shape::Pair(1, "a".into()),
                    Shape::Named { x: 1, y: 2 },
                    vec![Shape::Unit, Shape::Wrap(1)],
                    Tagged::A { x: 1 },
                    Tagged::B,
                    Adjacent::A(1),
                    Adjacent::B,
                    Either::L(vec![Either::N(1), Either::S("s".into())]),
                    Newtype(3),
                    Pair(1, 2),
                    Unit,
                    Ok::<i32, String>(1),
                    Err::<i32, String>("e".into()),
                    BTreeMap::from([(1u32, "a"), (10, "b"), (2, "c")]),
                    BTreeMap::from([(true, 1), (false, 2)]),
                    BTreeMap::from([(Key::Unit, 1), (Key::B, 2)]),
                    BTreeMap::from([('c', 1)]),
                    BTreeMap::from([(Some(1), 1)]),
                    BTreeMap::from([("b", 1), ("a", 2), ("__proto__", 3), ("0", 4), ("01", 5)]),
                    BTreeMap::from([((), 1)]),
                    BTreeMap::from([(Shape::Wrap(1), 1)]),
                    FloatKeys(&[(1.5, 1), (-0.0, 2), (1e21, 3)]),
                    FloatKeys(&[(f64::NAN, 1)]),
                    Keyed(&[("x", 1), ("y", 2), ("x", 3)]),
                    Keyed(&[(1u8, "a"), (1, "b")]),
                    Fails,
                    vec![Some(Fails)],
                    RawBytes(&[1, 2, 255]),
                    serde_json::json!({ "b": [1, 2.5, null], "a": { "c": "d" } }),
                    RawValue::from_string(r#"{ "x": [1, 2.50] }"#.into()).unwrap(),
                );
                m
            })
        });
        assert!(
            mismatches.is_empty(),
            "{} differences:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}
