//! Native module values: engine values read straight into serde types and serde output built
//! straight into engine values, with the meaning a JSON round trip gives them.

use std::borrow::Cow;
use std::fmt;

use serde::de::{self, IntoDeserializer, Unexpected, Visitor};
use serde::ser::{self, Serialize};
use serde_json::Value as Json;

use crate::js::{Context, Value};

#[derive(Debug)]
pub(crate) enum Error {
    /// What serde reports; the caller prefixes the argument.
    Message(String),
    /// A JS exception (a getter, `toJSON`, a BigInt), rethrown as it is.
    Thrown(Value),
    /// The arguments are not JSON at all.
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

fn json_error(e: serde_json::Error) -> Error {
    Error::Message(e.to_string())
}

fn engine(e: crate::js::Error) -> Error {
    match e {
        crate::js::Error::Exception(v) => Error::Thrown(v),
        other => Error::Message(other.to_string()),
    }
}

thread_local! {
    static JSON: std::cell::Cell<Option<(Value, Value)>> = const { std::cell::Cell::new(None) };
}

/// `JSON.parse` and `JSON.stringify` as the realm had them at install, protected for good.
pub(crate) fn install(context: &Context) -> crate::js::Result<()> {
    let json = context.global_get("JSON")?;
    let parse = context.get(json, &context.atom("parse"))?;
    let stringify = context.get(json, &context.atom("stringify"))?;
    context.protect(parse);
    context.protect(stringify);
    JSON.with(|j| j.set(Some((parse, stringify))));
    Ok(())
}

fn json_fns() -> Result<(Value, Value), Error> {
    JSON.with(|j| j.get())
        .ok_or_else(|| Error::Message("native modules are not installed".into()))
}

pub(crate) fn parse(context: &Context, text: &str) -> Result<Value, Error> {
    let (parse, _) = json_fns()?;
    context
        .call(parse, Value::undefined(), &[string(context, text)])
        .map_err(engine)
}

/// What JSON.stringify makes of `value` as an array element, read back by serde_json.
fn through_json(context: &Context, value: Value) -> Result<Json, Error> {
    let (_, stringify) = json_fns()?;
    let array = context.array(1).map_err(engine)?;
    context.set_index(array, 0, value).map_err(engine)?;
    let text = context
        .call(stringify, Value::undefined(), &[array])
        .map_err(engine)?;
    let text = context.read_string(text).map_err(engine)?;
    let mut list: Vec<Json> = serde_json::from_str(&text)
        .map_err(|e| Error::NotJson(format!("the arguments are not JSON: {e}")))?;
    Ok(list.pop().unwrap_or(Json::Null))
}

pub(crate) fn string(context: &Context, text: &str) -> Value {
    if text.is_ascii() {
        let mut out = 0u64;
        let status = unsafe {
            redjs::glue::rjs_string_latin1(context.raw(), text.as_ptr(), text.len(), &mut out)
        };
        if status == redjs::glue::OK {
            return Value(out);
        }
    }
    context.string(text)
}

/// A JS string's text; ASCII is borrowed from the engine's own storage.
///
/// # Safety
/// The result borrows the string cell: use it before anything can collect `value`.
unsafe fn read_str<'s>(context: &Context, value: Value) -> Option<Result<Cow<'s, str>, Error>> {
    let mut info = redjs::glue::RjsStr::default();
    if unsafe { redjs::glue::rjs_string_info(context.raw(), value.0, &mut info) } != redjs::glue::OK
    {
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
    for (i, c) in char::decode_utf16(units.iter().copied()).enumerate() {
        match c {
            Ok(c) => out.push(c),
            Err(e) => {
                let unit = e.unpaired_surrogate();
                let at = units[i..]
                    .iter()
                    .position(|&u| u == unit)
                    .map_or(0, |p| p + i);
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
                return Err(Error::NotJson(format!(
                    "the arguments are not JSON: {message}"
                )));
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
    if n.fract() == 0.0 && n.abs() < 1e21 {
        if n >= 0.0 {
            if n < 18_446_744_073_709_551_616.0 {
                return visitor.visit_u64(n as u64);
            }
        } else if n >= -9_223_372_036_854_775_808.0 {
            return visitor.visit_i64(n as i64);
        }
    }
    visitor.visit_f64(n)
}

fn number_unexpected(n: f64) -> Unexpected<'static> {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        if (0.0..18_446_744_073_709_551_616.0).contains(&n) {
            return Unexpected::Unsigned(n as u64);
        }
        if n < 0.0 && n >= -9_223_372_036_854_775_808.0 {
            return Unexpected::Signed(n as i64);
        }
    }
    Unexpected::Float(n)
}

/// A macro-generated `&str` parameter: the string read in place for the length of one call.
#[doc(hidden)]
pub struct Str {
    borrowed: (*const u8, usize),
    owned: Option<String>,
}

pub(crate) const STR_TOKEN: &str = "\0skathi::Str";

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

/// Reads one argument the way `JSON.stringify` then serde_json would.
pub(crate) struct Arg<'a> {
    context: &'a Context,
    value: Value,
}

impl<'a> Arg<'a> {
    pub(crate) fn new(context: &'a Context, value: Value) -> Self {
        Self { context, value }
    }
}

enum Prim<'s> {
    Null,
    Bool(bool),
    Number(f64),
    Str(Cow<'s, str>),
    Other,
}

impl<'a> Arg<'a> {
    fn prim<'s>(&self) -> Result<Prim<'s>, Error> {
        let v = self.value;
        if v.is_undefined() || v.is_null() {
            return Ok(Prim::Null);
        }
        if let Some(b) = v.as_bool() {
            return Ok(Prim::Bool(b));
        }
        if let Some(n) = v.as_number() {
            return Ok(if n.is_finite() {
                Prim::Number(n)
            } else {
                Prim::Null
            });
        }
        // SAFETY: the argument is on the caller's frame for the whole call.
        match unsafe { read_str(self.context, v) } {
            Some(s) => s.map(Prim::Str),
            None => Ok(Prim::Other),
        }
    }

    fn json(&self) -> Result<Json, Error> {
        through_json(self.context, self.value)
    }
}

fn unexpected<'s>(p: &'s Prim<'_>) -> Unexpected<'s> {
    match p {
        Prim::Null | Prim::Other => Unexpected::Unit,
        Prim::Bool(b) => Unexpected::Bool(*b),
        Prim::Number(n) => number_unexpected(*n),
        Prim::Str(s) => Unexpected::Str(s),
    }
}

macro_rules! through_json {
    ($self:ident, $method:ident($($arg:expr),*)) => {
        de::Deserializer::$method($self.json()?, $($arg),*).map_err(json_error)
    };
}

macro_rules! scalar {
    ($($method:ident => |$p:ident, $visitor:ident| $body:expr;)*) => {
        $(fn $method<V: Visitor<'de>>(self, $visitor: V) -> Result<V::Value, Error> {
            match self.prim()? {
                Prim::Other => through_json!(self, $method($visitor)),
                $p => $body,
            }
        })*
    };
}

macro_rules! numbers {
    ($($method:ident)*) => {
        scalar! {
            $($method => |p, visitor| match p {
                Prim::Number(n) => visit_number(n, visitor),
                p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
            };)*
        }
    };
}

macro_rules! strings {
    ($($method:ident)*) => {
        scalar! {
            $($method => |p, visitor| match p {
                Prim::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
                Prim::Str(Cow::Owned(s)) => visitor.visit_string(s),
                p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
            };)*
        }
    };
}

macro_rules! compound {
    ($($method:ident($($name:ident: $ty:ty),*);)*) => {
        $(fn $method<V: Visitor<'de>>(self, $($name: $ty,)* visitor: V) -> Result<V::Value, Error> {
            match self.prim()? {
                Prim::Other => through_json!(self, $method($($name,)* visitor)),
                p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
            }
        })*
    };
}

impl<'de, 'a> de::Deserializer<'de> for Arg<'a> {
    type Error = Error;

    scalar! {
        deserialize_any => |p, visitor| match p {
            Prim::Null => visitor.visit_unit(),
            Prim::Bool(b) => visitor.visit_bool(b),
            Prim::Number(n) => visit_number(n, visitor),
            Prim::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
            Prim::Str(Cow::Owned(s)) => visitor.visit_string(s),
            Prim::Other => unreachable!(),
        };
        deserialize_bool => |p, visitor| match p {
            Prim::Bool(b) => visitor.visit_bool(b),
            p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
        };
        deserialize_unit => |p, visitor| match p {
            Prim::Null => visitor.visit_unit(),
            p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
        };
        deserialize_bytes => |p, visitor| match p {
            Prim::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
            Prim::Str(Cow::Owned(s)) => visitor.visit_string(s),
            p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
        };
        deserialize_byte_buf => |p, visitor| match p {
            Prim::Str(Cow::Borrowed(s)) => visitor.visit_str(s),
            Prim::Str(Cow::Owned(s)) => visitor.visit_string(s),
            p => Err(de::Error::invalid_type(unexpected(&p), &visitor)),
        };
        deserialize_ignored_any => |p, visitor| {
            let _ = p;
            visitor.visit_unit()
        };
    }

    numbers! {
        deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64 deserialize_i128
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64 deserialize_u128
        deserialize_f32 deserialize_f64
    }

    strings! {
        deserialize_char deserialize_str deserialize_string deserialize_identifier
    }

    compound! {
        deserialize_seq();
        deserialize_tuple(len: usize);
        deserialize_tuple_struct(name: &'static str, len: usize);
        deserialize_map();
        deserialize_struct(name: &'static str, fields: &'static [&'static str]);
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.prim()? {
            Prim::Null => visitor.visit_none(),
            Prim::Other => through_json!(self, deserialize_option(visitor)),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.prim()? {
            Prim::Str(s) => visitor.visit_enum(s.into_owned().into_deserializer()),
            Prim::Other => through_json!(self, deserialize_enum(name, variants, visitor)),
            p => Err(de::Error::invalid_type(unexpected(&p), &"string or map")),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        if name == STR_TOKEN {
            if let Prim::Str(Cow::Borrowed(s)) = self.prim()? {
                // SAFETY: the engine's string outlives the call that reads it, which is as long
                // as a `Str` is used.
                let s: &'de str = unsafe { &*(s as *const str) };
                return visitor.visit_borrowed_str(s);
            }
            return self.deserialize_string(visitor);
        }
        if name == "$serde_json::private::RawValue" {
            return through_json!(self, deserialize_newtype_struct(name, visitor));
        }
        visitor.visit_newtype_struct(self)
    }
}

/// Builds a result: scalars straight into engine values; anything else stops at once and is
/// built by the JSON round trip.
pub(crate) struct Out<'a> {
    pub(crate) context: &'a Context,
    pub(crate) top: bool,
}

#[derive(Debug)]
pub(crate) enum OutError {
    Compound,
    Message(String),
    /// The top-level `Err(e)`, as serde_json writes `e`.
    Err(String),
}

impl fmt::Display for OutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutError::Compound => f.write_str("a compound value"),
            OutError::Message(m) | OutError::Err(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for OutError {}

impl ser::Error for OutError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        OutError::Message(msg.to_string())
    }
}

/// How JSON.parse reads what serde_json writes for an `f32`: its shortest decimal form.
pub(crate) fn f32_number(v: f32) -> f64 {
    if !v.is_finite() {
        return f64::NAN;
    }
    zmij::Buffer::new()
        .format_finite(v)
        .parse()
        .unwrap_or(v as f64)
}

pub(crate) fn number(v: f64) -> Value {
    if v.is_finite() {
        Value::number(v)
    } else {
        Value::null()
    }
}

type Impossible = ser::Impossible<Value, OutError>;

impl<'a> ser::Serializer for Out<'a> {
    type Ok = Value;
    type Error = OutError;
    type SerializeSeq = Impossible;
    type SerializeTuple = Impossible;
    type SerializeTupleStruct = Impossible;
    type SerializeTupleVariant = Impossible;
    type SerializeMap = Impossible;
    type SerializeStruct = Impossible;
    type SerializeStructVariant = Impossible;

    fn serialize_bool(self, v: bool) -> Result<Value, OutError> {
        Ok(Value::boolean(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Value, OutError> {
        Ok(Value::int32(v as i32))
    }
    fn serialize_i16(self, v: i16) -> Result<Value, OutError> {
        Ok(Value::int32(v as i32))
    }
    fn serialize_i32(self, v: i32) -> Result<Value, OutError> {
        Ok(Value::int32(v))
    }
    fn serialize_i64(self, v: i64) -> Result<Value, OutError> {
        Ok(Value::number(v as f64))
    }
    fn serialize_i128(self, v: i128) -> Result<Value, OutError> {
        Ok(Value::number(v as f64))
    }
    fn serialize_u8(self, v: u8) -> Result<Value, OutError> {
        Ok(Value::int32(v as i32))
    }
    fn serialize_u16(self, v: u16) -> Result<Value, OutError> {
        Ok(Value::int32(v as i32))
    }
    fn serialize_u32(self, v: u32) -> Result<Value, OutError> {
        Ok(Value::number(v as f64))
    }
    fn serialize_u64(self, v: u64) -> Result<Value, OutError> {
        Ok(Value::number(v as f64))
    }
    fn serialize_u128(self, v: u128) -> Result<Value, OutError> {
        Ok(Value::number(v as f64))
    }
    fn serialize_f32(self, v: f32) -> Result<Value, OutError> {
        Ok(number(f32_number(v)))
    }
    fn serialize_f64(self, v: f64) -> Result<Value, OutError> {
        Ok(number(v))
    }
    fn serialize_char(self, v: char) -> Result<Value, OutError> {
        Ok(string(self.context, v.encode_utf8(&mut [0; 4])))
    }
    fn serialize_str(self, v: &str) -> Result<Value, OutError> {
        Ok(string(self.context, v))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Value, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_none(self) -> Result<Value, OutError> {
        Ok(Value::null())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<Value, OutError> {
        v.serialize(Out { top: false, ..self })
    }
    fn serialize_unit(self) -> Result<Value, OutError> {
        Ok(Value::null())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Value, OutError> {
        Ok(Value::null())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Value, OutError> {
        Ok(string(self.context, variant))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<Value, OutError> {
        v.serialize(Out { top: false, ..self })
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<Value, OutError> {
        if self.top && name == "Result" {
            match variant {
                "Ok" => return v.serialize(Out { top: false, ..self }),
                "Err" => {
                    return Err(match serde_json::to_string(v) {
                        Ok(json) => OutError::Err(json),
                        Err(e) => OutError::Message(e.to_string()),
                    })
                }
                _ => {}
            }
        }
        Err(OutError::Compound)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_tuple(self, _: usize) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Impossible, OutError> {
        Err(OutError::Compound)
    }
}
