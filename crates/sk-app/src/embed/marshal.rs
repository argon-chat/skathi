//! Native module values, read from the engine straight into serde types and built from serde
//! output straight into engine values, meaning what a JSON round trip (`JSON.stringify`, then
//! serde_json; serde_json, then `JSON.parse`) means.

use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::c_void;
use std::fmt;

use redjs::glue::{self, json};
use serde::de::{self, DeserializeSeed, IntoDeserializer, Unexpected, Visitor};
use serde::ser::{self, Serialize};
use serde_json::Value as Json;

use super::native_buffer;
use crate::js::{Context, TaKind, Value};
use crate::stdlib::memory::Region;

mod direct;
pub use direct::*;

#[derive(Debug)]
#[doc(hidden)]
pub enum Error {
    /// What serde reports; the caller prefixes the argument.
    Message(String),
    /// A JS exception (a getter, `toJSON`, a BigInt, a cycle), rethrown as it is.
    Thrown(Value),
    /// What serde_json refused to parse in the JSON round trip.
    NotJson(String),
    /// An object or an array, which a result builds in one engine call instead.
    Compound,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Message(m) | Error::NotJson(m) => f.write_str(m),
            Error::Thrown(_) => f.write_str("a JS exception"),
            Error::Compound => f.write_str("an object or an array"),
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
}

thread_local! {
    static INTRINSICS: std::cell::Cell<Option<Intrinsics>> = const { std::cell::Cell::new(None) };
    static TO_JSON: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    static SCRATCH: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) };
}

pub(crate) fn install(context: &Context) -> crate::js::Result<()> {
    let json = context.global_get("JSON")?;
    let intrinsics = Intrinsics {
        parse: context.get(json, &context.atom("parse"))?,
        stringify: context.get(json, &context.atom("stringify"))?,
    };
    context.protect(intrinsics.parse);
    context.protect(intrinsics.stringify);
    INTRINSICS.with(|i| i.set(Some(intrinsics)));
    Ok(())
}

fn intrinsics() -> Result<Intrinsics, Error> {
    INTRINSICS
        .with(|i| i.get())
        .ok_or_else(|| Error::Message("native modules are not installed".into()))
}

/// An atom made once per realm and kept: freeing one needs the engine's lock.
fn atom(context: &Context, name: &str) -> *mut c_void {
    // SAFETY: plain FFI; the name is read during the call.
    unsafe { glue::rjs_atom8(context.raw(), name.as_ptr(), name.len()) }
}

fn to_json(context: &Context) -> *mut c_void {
    TO_JSON.with(|cell| {
        let (realm, a) = cell.get();
        if realm == context.raw() as usize && a != 0 {
            return a as *mut c_void;
        }
        let a = atom(context, "toJSON");
        cell.set((context.raw() as usize, a as usize));
        a
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
    Some(unsafe { text(info.ptr, info.len, info.is8bit != 0) })
}

/// A string's characters, Latin-1 or UTF-16; ASCII is borrowed.
///
/// # Safety
/// `ptr` holds `len` units of a string cell that stays alive while the result is used.
unsafe fn text<'s>(ptr: *const c_void, len: u32, latin1: bool) -> Result<Cow<'s, str>, Error> {
    if len == 0 {
        return Ok(Cow::Borrowed(""));
    }
    let len = len as usize;
    if latin1 {
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
        if bytes.is_ascii() {
            Ok(Cow::Borrowed(unsafe {
                std::str::from_utf8_unchecked(bytes)
            }))
        } else {
            Ok(Cow::Owned(bytes.iter().map(|&b| b as char).collect()))
        }
    } else {
        let units = unsafe { std::slice::from_raw_parts(ptr as *const u16, len) };
        from_utf16(units).map(Cow::Owned)
    }
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

/// Element kinds of `rjs_typedarray_info`; an `ArrayBuffer` reads as a `DataView`.
const TA_U8: u32 = 2;
const TA_BIG_I64: u32 = 10;
const TA_BIG_U64: u32 = 11;
const TA_DATAVIEW: u32 = 12;
const TA_F16: u32 = 13;

/// The memory of an `ArrayBufferView` or an `ArrayBuffer`.
#[derive(Clone, Copy)]
struct Buffer {
    data: *const u8,
    len: usize,
    kind: u32,
    shared: bool,
}

impl Buffer {
    /// # Safety
    /// While the buffer's owner is alive and JS has not run.
    unsafe fn bytes<'s>(&self) -> &'s [u8] {
        if self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.data, self.len) }
    }
}

/// # Safety
/// `len` bytes at `ptr` stay allocated for `'s`.
unsafe fn slice_at<'s>(ptr: *const u8, len: usize) -> &'s [u8] {
    if len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

fn buffer(context: &Context, v: Value) -> Option<Buffer> {
    if !v.is_cell() {
        return None;
    }
    match context.kind(v) {
        11 => {
            let info = context.typedarray_info(v).ok()?;
            Some(Buffer {
                data: info.data as *const u8,
                len: if info.detached != 0 {
                    0
                } else {
                    info.byte_length as usize
                },
                kind: info.kind,
                shared: info.shared != 0,
            })
        }
        12 => {
            let info = context.arraybuffer_info(v).ok()?;
            Some(Buffer {
                data: info.data as *const u8,
                len: if info.detached != 0 {
                    0
                } else {
                    info.byte_length as usize
                },
                kind: TA_DATAVIEW,
                shared: info.shared != 0,
            })
        }
        _ => None,
    }
}

/// A number type a typed array holds, for `&[T]` parameters.
#[doc(hidden)]
pub trait Element: Copy + for<'de> de::Deserialize<'de> + 'static {
    const TOKEN: &'static str;
    #[cfg(feature = "types")]
    const ARRAY: &'static str;
}

macro_rules! elements {
    ($($t:ty => $kind:literal $array:literal,)*) => {
        $(impl Element for $t {
            const TOKEN: &'static str = concat!("\0skathi::Slice<", stringify!($t), ">");
            #[cfg(feature = "types")]
            const ARRAY: &'static str = $array;
        })*

        /// The typed array kind a `&[T]` borrows from, by `T`'s token.
        fn slice_kind(token: &str) -> Option<u32> {
            $(if token == <$t as Element>::TOKEN {
                return Some($kind);
            })*
            None
        }
    };
}

elements! {
    u8 => 2 "Uint8Array",
    i8 => 1 "Int8Array",
    i16 => 4 "Int16Array",
    u16 => 5 "Uint16Array",
    i32 => 6 "Int32Array",
    u32 => 7 "Uint32Array",
    f32 => 8 "Float32Array",
    f64 => 9 "Float64Array",
    i64 => 10 "BigInt64Array",
    u64 => 11 "BigUint64Array",
}

/// A macro-generated `&[T]` parameter: the typed array read in place for the length of one call,
/// or a copy of what else was passed.
#[doc(hidden)]
pub struct Slice<T: Element> {
    borrowed: (*const T, usize),
    owned: Option<Vec<T>>,
}

impl<T: Element> Slice<T> {
    /// # Safety
    /// Only inside the call whose arguments produced it.
    pub unsafe fn get(&self) -> &[T] {
        match &self.owned {
            Some(v) => v,
            None if self.borrowed.1 == 0 => &[],
            None => unsafe { std::slice::from_raw_parts(self.borrowed.0, self.borrowed.1) },
        }
    }
}

impl<'de, T: Element> de::Deserialize<'de> for Slice<T> {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SliceVisitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Element> Visitor<'de> for SliceVisitor<T> {
            type Value = Slice<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a typed array")
            }
            fn visit_borrowed_bytes<E: de::Error>(self, v: &'de [u8]) -> Result<Slice<T>, E> {
                Ok(Slice {
                    borrowed: (v.as_ptr() as *const T, v.len() / std::mem::size_of::<T>()),
                    owned: None,
                })
            }
            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Slice<T>, E> {
                if std::mem::size_of::<T>() != 1 {
                    return Err(E::invalid_type(Unexpected::Bytes(&v), &self));
                }
                let mut v = std::mem::ManuallyDrop::new(v);
                // SAFETY: `T` is `u8` or `i8`, the same layout as the `u8` buffer.
                let owned =
                    unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut T, v.len(), v.capacity()) };
                Ok(Slice {
                    borrowed: (std::ptr::null(), 0),
                    owned: Some(owned),
                })
            }
            fn visit_newtype_struct<D: de::Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<Slice<T>, D::Error> {
                Ok(Slice {
                    borrowed: (std::ptr::null(), 0),
                    owned: Some(<Vec<T> as de::Deserialize>::deserialize(d)?),
                })
            }
        }
        deserializer.deserialize_newtype_struct(T::TOKEN, SliceVisitor(std::marker::PhantomData))
    }
}

#[cfg(feature = "types")]
impl<T: Element> specta::Type for Slice<T> {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define(T::ARRAY).into()
    }
}

const SLICE_MUT_TOKEN: &str = "\0skathi::SliceMut<u8>";

thread_local! {
    /// The memory a `SliceMut` is being read from, handed to its visitor.
    static WRITABLE: std::cell::Cell<Option<(*mut u8, usize)>> = const { std::cell::Cell::new(None) };
}

/// A macro-generated `&mut [u8]` parameter: the memory of a view, an `ArrayBuffer` or a
/// `NativeBuffer`, written in place for the length of one call. Never a copy.
#[doc(hidden)]
pub struct SliceMut {
    borrowed: (*mut u8, usize),
}

impl SliceMut {
    /// # Safety
    /// Only inside the call whose arguments produced it, and only once.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get(&self) -> &mut [u8] {
        match self.borrowed {
            (_, 0) => &mut [],
            (ptr, len) => unsafe { std::slice::from_raw_parts_mut(ptr, len) },
        }
    }
}

impl<'de> de::Deserialize<'de> for SliceMut {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SliceMutVisitor;
        impl<'de> Visitor<'de> for SliceMutVisitor {
            type Value = SliceMut;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a typed array, a DataView, an ArrayBuffer or a NativeBuffer")
            }
            fn visit_unit<E: de::Error>(self) -> Result<SliceMut, E> {
                match WRITABLE.take() {
                    Some(borrowed) => Ok(SliceMut { borrowed }),
                    None => Err(E::invalid_type(Unexpected::Unit, &self)),
                }
            }
        }
        deserializer.deserialize_newtype_struct(SLICE_MUT_TOKEN, SliceMutVisitor)
    }
}

#[cfg(feature = "types")]
impl specta::Type for SliceMut {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("Uint8Array").into()
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
            pre: std::cell::Cell::new(None),
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
    Atom(*mut c_void),
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
#[doc(hidden)]
pub struct Js<'w, 'a> {
    walk: &'w Walk<'a>,
    key: Key,
    value: Value,
    /// The node already made of `value`: `toJSON` runs once per value.
    pre: std::cell::Cell<Option<Node<'static>>>,
}

impl<'w, 'a> Js<'w, 'a> {
    fn context(&self) -> &'a Context {
        self.walk.context
    }

    /// SerializeJSONProperty: `None` is what an object leaves out and an array writes as null.
    fn node<'s>(&self) -> Result<Option<Node<'s>>, Error> {
        if let Some(node) = self.pre.take() {
            return Ok(Some(node));
        }
        let v = self.value;
        if let Some(node) = scalar(v) {
            return Ok(node);
        }
        let context = self.context();
        let (key, atom, index) = match self.key {
            Key::Index(i) => (0, std::ptr::null_mut(), i),
            Key::Name(name) => (name.0, std::ptr::null_mut(), 0),
            Key::Atom(atom) => (0, atom, 0),
        };
        let mut out = glue::RjsJsonNode::default();
        // SAFETY: plain FFI on a live value; the atoms are kept for the realm's life.
        let status = unsafe {
            glue::rjs_json_node(
                context.raw(),
                v.0,
                to_json(context),
                key,
                atom,
                index,
                &mut out,
            )
        };
        match status {
            glue::OK => {}
            glue::EXCEPTION => return Err(Error::Thrown(Value(out.value))),
            _ => return Err(Error::Message("a value cannot be read".into())),
        }
        // SAFETY: a string's characters are `out.value`'s. That is `v`, held by the caller, or a
        // string toJSON or a String object made, copied before it can be collected.
        unsafe { made(context, &out, out.value != v.0) }
    }

    fn require<'s>(&self) -> Result<Node<'s>, Error> {
        Ok(self.node()?.unwrap_or(Node::Null))
    }

    /// A `&mut [u8]` parameter: memory written in place, never a copy.
    fn writable<'de, V: Visitor<'de>>(&self, visitor: V) -> Result<V::Value, Error> {
        if !native_buffer::in_call() {
            return Err(Error::Message(
                "a &mut [u8] is read only for a native module function's call".into(),
            ));
        }
        let found = match buffer(self.context(), self.value) {
            Some(b) if b.shared => {
                return Err(Error::Message(
                    "a SharedArrayBuffer cannot be written in place: another thread may use it"
                        .into(),
                ))
            }
            Some(b) => {
                native_buffer::borrow(b.data, b.len, true)?;
                Some((b.data as *mut u8, b.len))
            }
            None => native_buffer::lent_bytes(self.context(), self.value, true)?,
        };
        let Some(found) = found else {
            let node = self.require()?;
            return Err(de::Error::invalid_type(unexpected(&node), &visitor));
        };
        WRITABLE.set(Some(found));
        let read = visitor.visit_unit();
        WRITABLE.set(None);
        read
    }

    fn json(&self) -> Result<Json, Error> {
        through_json(self.context(), self.value)
    }

    fn child(&self, key: Key, value: Value) -> Js<'w, 'a> {
        Js {
            walk: self.walk,
            key,
            value,
            pre: std::cell::Cell::new(None),
        }
    }

    fn child_node(&self, key: Key, value: Value, node: Node<'static>) -> Js<'w, 'a> {
        let child = self.child(key, value);
        child.pre.set(Some(node));
        child
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

    /// An array's elements, each read by `read`.
    fn items<T>(
        &self,
        array: Value,
        mut read: impl FnMut(Js<'w, 'a>) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        const CHUNK: usize = 32;
        let _entered = self.walk.enter(array)?;
        let context = self.context();
        let len = context.array_length(array).map_err(engine)?;
        let mut out = Vec::with_capacity(len.min(1 << 16) as usize);
        // On this stack, which the collector scans: the values stay alive while they are read.
        let mut nodes = [glue::RjsJsonNode::default(); CHUNK];
        let mut at = 0;
        while at < len {
            let mut count = 0u32;
            // SAFETY: plain FFI on a live array; `nodes` has room for CHUNK.
            let status = unsafe {
                glue::rjs_json_items(
                    context.raw(),
                    array.0,
                    at,
                    len,
                    to_json(context),
                    nodes.as_mut_ptr(),
                    CHUNK as u32,
                    &mut count,
                )
            };
            match status {
                glue::OK => {}
                glue::EXCEPTION => return Err(Error::Thrown(Value(nodes[0].value))),
                _ => return Err(Error::Message("a value cannot be read".into())),
            }
            for raw in &nodes[..count as usize] {
                // SAFETY: a string's characters belong to `raw.value`, alive in `nodes`; they
                // are copied, since a later element's toJSON may drop the array's hold on it.
                let node = unsafe { made(context, raw, true) }?.unwrap_or(Node::Null);
                out.push(read(self.child_node(
                    Key::Index(at),
                    Value(raw.value),
                    node,
                ))?);
                at += 1;
            }
        }
        Ok(out)
    }

    /// An object's entries as JSON.stringify writes them; `false` for anything else.
    fn entries(
        &self,
        mut each: impl FnMut(String, Js<'w, 'a>) -> Result<(), Error>,
    ) -> Result<bool, Error> {
        let node: Node<'static> = self.require()?;
        let Node::Object(object) = node else {
            self.pre.set(Some(node));
            return Ok(false);
        };
        let _entered = self.walk.enter(object)?;
        let mut map = Map::new(self, object)?;
        while map.advance()? {
            let context = self.context();
            // SAFETY: the key string is held by `map.keys` for the whole walk.
            let key = match unsafe { read_str(context, map.key) } {
                Some(key) => key?.into_owned(),
                None => return Err(Error::Message("a key is not a string".into())),
            };
            let Some((value, node)) = map.value.take() else {
                return Err(de::Error::custom("value is missing"));
            };
            each(key, self.child_node(Key::Name(map.key), value, node))?;
        }
        Ok(true)
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

/// The node `rjs_json_node` described; `copy` copies a string's characters.
///
/// # Safety
/// A string's characters must stay alive while a borrowed node is used.
unsafe fn made<'s>(
    context: &Context,
    out: &glue::RjsJsonNode,
    copy: bool,
) -> Result<Option<Node<'s>>, Error> {
    Ok(Some(match out.kind {
        json::ABSENT => return Ok(None),
        json::NULL => Node::Null,
        json::FALSE => Node::Bool(false),
        json::TRUE => Node::Bool(true),
        json::NUMBER => Node::Number(out.number),
        json::STRING8 | json::STRING16 => {
            // SAFETY: the caller's.
            let text = unsafe { text(out.ptr, out.len, out.kind == json::STRING8) }?;
            Node::Str(if copy {
                Cow::Owned(text.into_owned())
            } else {
                text
            })
        }
        json::ARRAY => Node::Array(Value(out.value)),
        json::BIGINT => return Err(stringify_error(context, Value(out.value))),
        _ => Node::Object(Value(out.value)),
    }))
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

/// A typed array, a DataView or an ArrayBuffer read as a sequence of its numbers.
fn elements<'de, V: Visitor<'de>>(b: Buffer, visitor: V) -> Result<V::Value, Error> {
    let width = match b.kind {
        TA_F16 => {
            return Err(de::Error::custom(
                "a Float16Array cannot be read: pass a Float32Array",
            ))
        }
        4 | 5 => 2,
        6..=8 => 4,
        9..=11 => 8,
        _ => 1,
    };
    let mut seq = Elements {
        b,
        width,
        next: 0,
        len: b.len / width,
    };
    let value = visitor.visit_seq(&mut seq)?;
    if seq.next < seq.len {
        return Err(de::Error::invalid_length(
            seq.len,
            &"fewer elements in array",
        ));
    }
    Ok(value)
}

struct Elements {
    b: Buffer,
    width: usize,
    next: usize,
    len: usize,
}

impl Elements {
    fn item(&self, i: usize) -> Item {
        // SAFETY: `i < len`, inside the buffer, which is held by the caller's frame and does not
        // change while no JS runs.
        unsafe {
            let p = self.b.data.add(i * self.width);
            match self.b.kind {
                1 => Item::Number(p.cast::<i8>().read() as f64),
                4 => Item::Number(p.cast::<i16>().read_unaligned() as f64),
                5 => Item::Number(p.cast::<u16>().read_unaligned() as f64),
                6 => Item::Number(p.cast::<i32>().read_unaligned() as f64),
                7 => Item::Number(p.cast::<u32>().read_unaligned() as f64),
                8 => Item::Number(p.cast::<f32>().read_unaligned() as f64),
                9 => Item::Number(p.cast::<f64>().read_unaligned()),
                TA_BIG_I64 => Item::I64(p.cast::<i64>().read_unaligned()),
                TA_BIG_U64 => Item::U64(p.cast::<u64>().read_unaligned()),
                _ => Item::U8(p.read()),
            }
        }
    }
}

impl<'de> de::SeqAccess<'de> for Elements {
    type Error = Error;

    #[inline]
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        if self.next >= self.len {
            return Ok(None);
        }
        let item = self.item(self.next);
        self.next += 1;
        seed.deserialize(item).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.len - self.next)
    }
}

/// One element of a typed array.
#[derive(Clone, Copy)]
enum Item {
    U8(u8),
    Number(f64),
    I64(i64),
    U64(u64),
}

impl<'de> de::Deserializer<'de> for Item {
    type Error = Error;

    #[inline]
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Item::U8(b) => visitor.visit_u8(b),
            Item::Number(n) => visit_number(n, visitor),
            Item::I64(i) => visitor.visit_i64(i),
            Item::U64(u) => visitor.visit_u64(u),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_some(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit
        unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier ignored_any
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
    value: Option<(Value, Node<'static>)>,
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
            let Some(node) = self.parent.child(Key::Name(key), value).node()? else {
                continue;
            };
            self.key = key;
            self.value = Some((value, node));
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
            Some((value, node)) => {
                seed.deserialize(self.parent.child_node(Key::Name(self.key), value, node))
            }
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
        if let Some(b) = buffer(self.context(), self.value) {
            // SAFETY: the view is an argument or a property value held by this frame, and the
            // visitor copies the bytes before any JS runs.
            return visitor.visit_bytes(unsafe { b.bytes() });
        }
        if let Some(region) = native_buffer::bytes_of(self.context(), self.value)? {
            // SAFETY: `region` holds the chunk, and the visitor copies the bytes before JS runs.
            return visitor.visit_bytes(unsafe { region.bytes() });
        }
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
            Some(node) => {
                self.pre.set(Some(node));
                visitor.visit_some(self)
            }
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
        if name == super::host_file::TOKEN {
            let path = unsafe {
                sk_runtime::redjs::webapi::file::path_of(self.context().raw(), self.value.0)
            }
            .ok_or_else(|| de::Error::custom("expected a File backed by a native path"))?;
            return visitor.visit_byte_buf(super::host_file::path_bytes(path));
        }
        if name == STR_TOKEN {
            let node: Node<'de> = self.require()?;
            if let Node::Str(Cow::Borrowed(s)) = node {
                // SAFETY: the engine's string outlives the call, which is as long as a `Str` is
                // used.
                return visitor.visit_borrowed_str(s);
            }
            // SAFETY: as above; the node is read again at once.
            self.pre.set(Some(unsafe {
                std::mem::transmute::<Node<'de>, Node<'static>>(node)
            }));
            return self.deserialize_string(visitor);
        }
        if name == RAW_VALUE_TOKEN {
            return de::Deserializer::deserialize_newtype_struct(self.json()?, name, visitor)
                .map_err(|e| Error::Message(e.to_string()));
        }
        if name == native_buffer::TOKEN {
            if native_buffer::read(self.context(), self.value)? {
                let read = visitor.visit_unit();
                native_buffer::discard_incoming();
                return read;
            }
            let node = self.require()?;
            return Err(de::Error::invalid_type(unexpected(&node), &visitor));
        }
        if name == SLICE_MUT_TOKEN {
            return self.writable(visitor);
        }
        if let Some(kind) = slice_kind(name) {
            if let Some(b) = buffer(self.context(), self.value) {
                // SAFETY: as for `Str`; a `&[u8]` may view any buffer's bytes, other slices
                // only their own element type, which typed arrays keep aligned.
                let bytes: &'de [u8] = unsafe { b.bytes() };
                if !b.shared && (kind == TA_U8 || b.kind == kind) {
                    native_buffer::borrow(b.data, b.len, false)?;
                    return visitor.visit_borrowed_bytes(bytes);
                }
                if kind == TA_U8 {
                    return visitor.visit_byte_buf(bytes.to_vec());
                }
            } else if kind == TA_U8 {
                if !native_buffer::in_call() {
                    if let Some(region) = native_buffer::bytes_of(self.context(), self.value)? {
                        // SAFETY: `region` holds the chunk while the bytes are copied.
                        return visitor.visit_byte_buf(unsafe { region.bytes() }.to_vec());
                    }
                } else if let Some((ptr, len)) =
                    native_buffer::lent_bytes(self.context(), self.value, false)?
                {
                    // SAFETY: the call keeps the chunk until it returns.
                    return visitor.visit_borrowed_bytes(unsafe { slice_at(ptr, len) });
                }
            }
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        if let Some(b) = buffer(self.context(), self.value) {
            return elements(b, visitor);
        }
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
                let value = value.map(|(v, node)| self.child_node(Key::Name(key), v, node));
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

/// Where a result is built: a scalar straight into an engine value, or anything into an
/// [`Owned`] encoding, which the engine builds in one call. An object's key comes before its
/// value, so an encoding is written in one pass.
pub(crate) trait Build: Copy {
    type Value;
    type Array;
    type Object;
    fn null(self) -> Self::Value;
    fn bool(self, v: bool) -> Self::Value;
    fn number(self, v: f64) -> Self::Value;
    fn string(self, v: &str) -> Self::Value;
    fn raw_json(self, json: &str) -> Result<Self::Value, Error>;
    fn bytes(self, v: &[u8]) -> Result<Self::Value, Error>;
    /// A `NativeBuffer` over `region`.
    fn native(self, region: Region) -> Result<Self::Value, Error>;
    fn array(self, len: usize) -> Result<Self::Array, Error>;
    fn push(self, array: &mut Self::Array, v: Self::Value) -> Result<(), Error>;
    fn end_array(self, array: Self::Array) -> Self::Value;
    fn object(self) -> Result<Self::Object, Error>;
    fn field(self, object: &mut Self::Object, key: &'static str);
    fn entry(self, object: &mut Self::Object, key: String);
    fn put(self, object: &mut Self::Object, v: Self::Value) -> Result<(), Error>;
    fn end_object(self, object: Self::Object) -> Self::Value;
}

fn push_index(context: &Context, array: &mut (Value, u32), v: Value) -> Result<(), Error> {
    // SAFETY: plain FFI on live values.
    match unsafe { glue::rjs_define_index(context.raw(), array.0 .0, array.1, v.0) } {
        glue::OK => {
            array.1 += 1;
            Ok(())
        }
        _ => Err(Error::Message("the result cannot be built".into())),
    }
}

/// Scalars straight into engine values. An object or an array is [`Error::Compound`]: the
/// caller encodes it and has the engine build it in one call.
#[derive(Clone, Copy)]
pub(crate) struct Engine<'a>(pub(crate) &'a Context);

impl Build for Engine<'_> {
    type Value = Value;
    type Array = ();
    type Object = ();

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
    fn bytes(self, v: &[u8]) -> Result<Value, Error> {
        let array = self.0.typedarray(TaKind::U8, v.len()).map_err(engine)?;
        if !v.is_empty() {
            let info = self.0.typedarray_info(array).map_err(engine)?;
            // SAFETY: the new array's storage is `v.len()` bytes and nothing else sees it yet.
            unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), info.data as *mut u8, v.len()) };
        }
        Ok(array)
    }
    fn native(self, region: Region) -> Result<Value, Error> {
        native_buffer::to_js(self.0, region)
    }
    fn array(self, _: usize) -> Result<(), Error> {
        Err(Error::Compound)
    }
    fn push(self, _: &mut (), _: Value) -> Result<(), Error> {
        Err(Error::Compound)
    }
    fn end_array(self, _: ()) -> Value {
        Value::undefined()
    }
    fn object(self) -> Result<(), Error> {
        Err(Error::Compound)
    }
    fn field(self, _: &mut (), _: &'static str) {}
    fn entry(self, _: &mut (), _: String) {}
    fn put(self, _: &mut (), _: Value) -> Result<(), Error> {
        Err(Error::Compound)
    }
    fn end_object(self, _: ()) -> Value {
        Value::undefined()
    }
}

/// A value encoded in one buffer (and byte arrays in their own), for the engine to build in one
/// call on the JS thread: `rjs_json_build` reads the tags below.
#[derive(Debug, Default)]
#[doc(hidden)]
pub struct Owned {
    code: Vec<u8>,
    blobs: Vec<Blob>,
}

/// What a `BYTES` tag stands for: a `Uint8Array` of its own bytes, or a `NativeBuffer`.
enum Blob {
    Bytes(Vec<u8>),
    Native(Region),
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Blob::Bytes(v) => write!(f, "Bytes({})", v.len()),
            Blob::Native(r) => write!(f, "Native({})", r.len()),
        }
    }
}

const NULL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const NUMBER: u8 = 3;
const STRING: u8 = 4;
const JSON: u8 = 5;
const BYTES: u8 = 6;
const ARRAY: u8 = 7;
const OBJECT: u8 = 8;
const FIELD: u8 = 9;
const ENTRY: u8 = 10;

unsafe extern "C" fn free_bytes(bytes: *mut c_void, capacity: *mut c_void) {
    // SAFETY: `bytes` and `capacity` are a `Vec<u8>`'s, handed over by `uint8_array`.
    drop(unsafe { Vec::from_raw_parts(bytes as *mut u8, 0, capacity as usize) });
}

/// A `Uint8Array` over `v`'s own memory.
fn uint8_array(context: &Context, v: Vec<u8>) -> Result<Value, Error> {
    if v.is_empty() {
        return Engine(context).bytes(&[]);
    }
    let mut v = std::mem::ManuallyDrop::new(v);
    // SAFETY: the engine owns the memory from here and frees it through `free_bytes`; on failure
    // it is leaked rather than risk freeing it twice.
    unsafe {
        context.typedarray_external(
            TaKind::U8,
            v.as_mut_ptr() as *mut c_void,
            v.len(),
            Some(free_bytes),
            v.capacity() as *mut c_void,
        )
    }
    .map_err(engine)
}

/// Writes an [`Owned`].
#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct Tree<'a>(&'a RefCell<Owned>);

impl Tree<'_> {
    fn tag(self, tag: u8) {
        self.0.borrow_mut().code.push(tag);
    }

    fn text(self, tag: u8, v: &str) {
        let code = &mut self.0.borrow_mut().code;
        code.push(tag);
        code.extend_from_slice(&(v.len() as u32).to_le_bytes());
        code.extend_from_slice(v.as_bytes());
    }

    /// A tag and a count to fill in at the end.
    fn open(self, tag: u8) -> (usize, u32) {
        let code = &mut self.0.borrow_mut().code;
        code.push(tag);
        let at = code.len();
        code.extend_from_slice(&[0; 4]);
        (at, 0)
    }

    fn close(self, (at, count): (usize, u32)) {
        self.0.borrow_mut().code[at..at + 4].copy_from_slice(&count.to_le_bytes());
    }

    fn blob(self, blob: Blob) {
        let mut owned = self.0.borrow_mut();
        let index = owned.blobs.len() as u32;
        owned.blobs.push(blob);
        owned.code.push(BYTES);
        owned.code.extend_from_slice(&index.to_le_bytes());
    }
}

impl Build for Tree<'_> {
    type Value = ();
    type Array = (usize, u32);
    type Object = (usize, u32);

    fn null(self) {
        self.tag(NULL);
    }
    fn bool(self, v: bool) {
        self.tag(if v { TRUE } else { FALSE });
    }
    fn number(self, v: f64) {
        if !v.is_finite() {
            return self.tag(NULL);
        }
        let code = &mut self.0.borrow_mut().code;
        code.push(NUMBER);
        code.extend_from_slice(&v.to_le_bytes());
    }
    fn string(self, v: &str) {
        self.text(STRING, v);
    }
    fn raw_json(self, json: &str) -> Result<(), Error> {
        self.text(JSON, json);
        Ok(())
    }
    fn bytes(self, v: &[u8]) -> Result<(), Error> {
        self.blob(Blob::Bytes(v.to_vec()));
        Ok(())
    }
    fn native(self, region: Region) -> Result<(), Error> {
        self.blob(Blob::Native(region));
        Ok(())
    }
    fn array(self, _: usize) -> Result<(usize, u32), Error> {
        Ok(self.open(ARRAY))
    }
    fn push(self, array: &mut (usize, u32), _: ()) -> Result<(), Error> {
        array.1 += 1;
        Ok(())
    }
    fn end_array(self, array: (usize, u32)) {
        self.close(array);
    }
    fn object(self) -> Result<(usize, u32), Error> {
        Ok(self.open(OBJECT))
    }
    fn field(self, _: &mut (usize, u32), key: &'static str) {
        let code = &mut self.0.borrow_mut().code;
        code.push(FIELD);
        code.extend_from_slice(&(key.as_ptr() as usize).to_le_bytes());
        code.extend_from_slice(&key.len().to_le_bytes());
    }
    fn entry(self, _: &mut (usize, u32), key: String) {
        self.text(ENTRY, &key);
    }
    fn put(self, object: &mut (usize, u32), _: ()) -> Result<(), Error> {
        object.1 += 1;
        Ok(())
    }
    fn end_object(self, object: (usize, u32)) {
        self.close(object);
    }
}

fn build(context: &Context, code: &[u8], blobs: Vec<Blob>) -> Result<Value, Error> {
    let list = if blobs.is_empty() {
        Value::undefined()
    } else {
        let list = context.array(blobs.len() as u32).map_err(engine)?;
        let mut at = (list, 0);
        for b in blobs {
            let v = match b {
                Blob::Bytes(b) => uint8_array(context, b)?,
                Blob::Native(region) => native_buffer::to_js(context, region)?,
            };
            push_index(context, &mut at, v)?;
        }
        list
    };
    let mut out = 0u64;
    // SAFETY: plain FFI; `code` is read during the call and `list` is held by this frame.
    match unsafe {
        glue::rjs_json_build(context.raw(), code.as_ptr(), code.len(), list.0, &mut out)
    } {
        glue::OK => Ok(Value(out)),
        glue::EXCEPTION => Err(Error::Thrown(Value(out))),
        _ => Err(Error::Message("the result cannot be built".into())),
    }
}

impl Owned {
    pub(crate) fn to_js(self, context: &Context) -> Result<Value, Error> {
        build(context, &self.code, self.blobs)
    }
}

/// Serializes `value` off the JS thread.
pub(crate) fn owned<T: Serialize + ?Sized>(value: &T, top: bool) -> Result<Owned, OutError> {
    encode(|tree| value.serialize(Out { build: tree, top }))
}

/// What `write` puts in a new [`Owned`].
pub(crate) fn encode(
    write: impl FnOnce(Tree<'_>) -> Result<(), OutError>,
) -> Result<Owned, OutError> {
    let owned = RefCell::new(Owned {
        code: Vec::with_capacity(256),
        blobs: Vec::new(),
    });
    write(Tree(&owned))?;
    Ok(owned.into_inner())
}

/// `value` as a JS value: a scalar made directly, anything else encoded and built in one call.
pub(crate) fn to_js<T: Serialize + ?Sized>(
    context: &Context,
    value: &T,
    top: bool,
) -> Result<Value, OutError> {
    match value.serialize(Out {
        build: Engine(context),
        top,
    }) {
        Err(OutError::Compound) => {
            build_with(context, |tree| value.serialize(Out { build: tree, top }))
        }
        other => other,
    }
}

/// What `write` encodes, built into a JS value; the buffer is this thread's, kept between calls.
pub(crate) fn build_with(
    context: &Context,
    write: impl FnOnce(Tree<'_>) -> Result<(), OutError>,
) -> Result<Value, OutError> {
    let mut code = SCRATCH.take();
    code.clear();
    let owned = RefCell::new(Owned {
        code,
        blobs: Vec::new(),
    });
    let written = write(Tree(&owned));
    let Owned { code, blobs } = owned.into_inner();
    let value = written.and_then(|()| build(context, &code, blobs).map_err(OutError::from));
    if code.capacity() <= 1 << 20 {
        SCRATCH.set(code);
    }
    value
}

#[derive(Debug)]
#[doc(hidden)]
pub enum OutError {
    Message(String),
    /// The top-level `Err(e)`, as serde_json writes `e`.
    Err(String),
    Thrown(Value),
    /// See [`Error::Compound`].
    Compound,
}

impl fmt::Display for OutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutError::Message(m) | OutError::Err(m) => f.write_str(m),
            OutError::Thrown(_) => f.write_str("a JS exception"),
            OutError::Compound => f.write_str("an object or an array"),
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
            Error::Compound => OutError::Compound,
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
    /// `{ "Variant": … }` around a tuple or struct variant.
    outer: Option<B::Object>,
    raw: Option<String>,
}

impl<B: Build> Compound<B> {
    fn outer(build: B, variant: Option<&'static str>) -> Result<Option<B::Object>, OutError> {
        let Some(variant) = variant else {
            return Ok(None);
        };
        let mut outer = build.object()?;
        build.field(&mut outer, variant);
        Ok(Some(outer))
    }

    fn array(build: B, len: usize, variant: Option<&'static str>) -> Result<Self, OutError> {
        let outer = Self::outer(build, variant)?;
        Ok(Self {
            build,
            array: Some(build.array(len)?),
            object: None,
            outer,
            raw: None,
        })
    }

    fn object(build: B, variant: Option<&'static str>) -> Result<Self, OutError> {
        let outer = Self::outer(build, variant)?;
        Ok(Self {
            build,
            array: None,
            object: Some(build.object()?),
            outer,
            raw: None,
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
        let Some(object) = &mut self.object else {
            return Err(OutError::Message("not an object".into()));
        };
        self.build.field(object, key);
        let v = v.serialize(Out {
            build: self.build,
            top: false,
        })?;
        Ok(self.build.put(self.object.as_mut().unwrap(), v)?)
    }

    fn finish(self) -> Result<B::Value, OutError> {
        let value = match (self.array, self.object) {
            (Some(array), _) => self.build.end_array(array),
            (_, Some(object)) => self.build.end_object(object),
            _ => return Err(OutError::Message("an empty compound".into())),
        };
        match self.outer {
            Some(mut outer) => {
                self.build.put(&mut outer, value)?;
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
        let key = key.serialize(KeySer)?;
        match &mut self.object {
            Some(object) => {
                self.build.entry(object, key);
                Ok(())
            }
            None => Err(OutError::Message("not an object".into())),
        }
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), OutError> {
        let v = v.serialize(Out {
            build: self.build,
            top: false,
        })?;
        match &mut self.object {
            Some(object) => Ok(self.build.put(object, v)?),
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
        if self.raw.is_some() {
            let serde_json::Value::String(json) =
                serde_json::to_value(v).map_err(|e| OutError::Message(e.to_string()))?
            else {
                return Err(OutError::Message("a RawValue without its JSON".into()));
            };
            self.raw = Some(json);
            return Ok(());
        }
        self.field(key, v)
    }
    fn end(self) -> Result<B::Value, OutError> {
        if let Some(json) = self.raw {
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
        Ok(self.build.bytes(v)?)
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
        name: &'static str,
        v: &T,
    ) -> Result<B::Value, OutError> {
        if name == native_buffer::TOKEN {
            if let Some(region) = native_buffer::outgoing() {
                return Ok(self.build.native(region)?);
            }
        }
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
        let mut object = self.build.object()?;
        self.build.field(&mut object, variant);
        let inner = v.serialize(self.inner())?;
        self.build.put(&mut object, inner)?;
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
        if name == RAW_VALUE_TOKEN {
            return Ok(Compound {
                build: self.build,
                array: None,
                object: None,
                outer: None,
                raw: Some(String::new()),
            });
        }
        Compound::object(self.build, None)
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
        read_with(context, value, |js| T::deserialize(js))
    }

    fn read_with<T>(
        context: &Context,
        value: Value,
        read: impl FnOnce(Js<'_, '_>) -> Result<T, Error>,
    ) -> Result<T, String> {
        let walk = Walk::new(context);
        read(walk.arg(0, value)).map_err(|e| match e {
            Error::Thrown(e) => thrown(context, e),
            Error::NotJson(m) => format!("TypeError: {m}"),
            e => format!("argument 1: {e}"),
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
        "({ Type: 1, given: 'a', More: ['one', { two: { aB: 1 } }, { 'three!': 3 }] })",
        "({ Type: 1, given: 'a', More: [], Inner: { four: { x: 1, y: 2 } }, When: [1, 2] })",
        "({ Type: 1, given: 'a', More: [{ two: { a_b: 1 } }] })",
        "({ kind: 'A', x: 1 })",
        "({ kind: 'C', x: 1, y: 2 })",
        "({ data: new Uint8Array([1]), mask: null })",
        "({ data: [1, 2], mask: new Uint8Array([3]).buffer })",
        "({ data: 'x' })",
        "[{ x: 1, y: 2 }, { x: 3 }]",
        "[{ x: 1, y: 2 }, 'Unit', { Wrap: 1 }]",
        "(() => { const a = [1, { toJSON() { a[2] = 'x'; return 2; } }, 3]; return a; })()",
        "(() => { const a = [1, 2]; Object.defineProperty(a, 0, { get() { a[1] = 'y'; return 0; }, enumerable: true }); return a; })()",
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

    /// Every value of the corpus read as `T` through serde over values and through `picked`,
    /// which is `FromJs` where `T` has it.
    fn arguments<T: DeserializeOwned + Debug + PartialEq>(
        context: &Context,
        name: &str,
        picked: impl Fn(Js<'_, '_>) -> Result<T, Error>,
        mismatches: &mut Vec<String>,
    ) {
        for src in VALUES {
            let before = through_json_then::<T>(context, eval(context, src));
            let now = direct::<T>(context, eval(context, src));
            let from_js = read_with(context, eval(context, src), &picked);
            // A value that is not JSON as a whole still fails, but reading stops at the first
            // problem, so a wrong type met before it is what gets reported.
            let whole = |e: &String| !e.starts_with("argument 1: ");
            // Sequences and bytes read typed arrays and buffers as their elements, which JSON
            // turned into objects with index keys.
            let memory = buffer(context, eval(context, src)).is_some();
            for (way, now, by_fields) in [("direct", &now, false), ("FromJs", &from_js, true)] {
                let agree = match (&before, now) {
                    (Err(old), Err(_)) if whole(old) => true,
                    // A struct read by its fields does not look at the others.
                    (Err(old), Ok(_)) if by_fields && whole(old) => true,
                    (Err(_), _) if memory => true,
                    _ => before == *now,
                };
                if !agree {
                    mismatches.push(format!(
                        "{name} from {src}:\n  JSON:   {before:?}\n  {way}: {now:?}"
                    ));
                }
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
                        $(arguments::<$t>(
                            context,
                            stringify!($t),
                            |js| (&&Pick::<$t>::new()).read(js),
                            &mut m,
                        );)*
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
                #[cfg(feature = "types")]
                {
                    use derived::*;
                    check!(
                        Point, Option<Point>, Vec<Point>, BTreeMap<String, Point>, Defaults,
                        Camel, Strict, Flat, Nested, Shape, Option<Shape>, Vec<Shape>, Tagged,
                        Adjacent, Either, Newtype, Pair, Unit, Renamed, Internal,
                    );
                }
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

    /// `value` built through serde and through `picked`, which is `IntoJs` where `T` has it.
    fn result<T: Serialize>(
        context: &Context,
        name: &str,
        value: T,
        picked: impl Fn(&T, Tree<'_>) -> Result<(), OutError>,
        mismatches: &mut Vec<String>,
    ) {
        let before = serde_json::to_string(&value)
            .map_err(|e| e.to_string())
            .map(|json| parse(context, &json).unwrap());
        let now = to_js(context, &value, false).map_err(|e| e.to_string());
        let owned = owned(&value, false)
            .map_err(|e| e.to_string())
            .map(|tree| tree.to_js(context).unwrap());
        let into_js = build_with(context, |tree| picked(&value, tree)).map_err(|e| e.to_string());
        let into_js_owned = encode(|tree| picked(&value, tree))
            .map_err(|e| e.to_string())
            .map(|tree| tree.to_js(context).unwrap());
        let same = intrinsics_same(context);
        for (way, got) in [
            ("direct", now),
            ("owned", owned),
            ("IntoJs", into_js),
            ("IntoJs owned", into_js_owned),
        ] {
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
                    ($($v:expr_2021),* $(,)?) => {
                        $({
                            let value = $v;
                            let pick = Pick::of(&value);
                            result(
                                context,
                                stringify!($v),
                                value,
                                |v, tree| (&&pick).encode(v, tree),
                                &mut m,
                            );
                        })*
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
                    serde_json::json!({ "b": [1, 2.5, null], "a": { "c": "d" } }),
                    RawValue::from_string(r#"{ "x": [1, 2.50] }"#.into()).unwrap(),
                );
                #[cfg(feature = "types")]
                {
                    use derived::*;
                    check!(
                        Point { x: 1, y: -2 },
                        vec![Point { x: 1, y: 2 }, Point { x: 3, y: 4 }],
                        BTreeMap::from([("a", Point { x: 1, y: 2 })]),
                        Some(Point { x: 5, y: 6 }),
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
                        Either::P(Point { x: 1, y: 2 }),
                        Newtype(3),
                        Pair(1, 2),
                        Unit,
                        Renamed {
                            r#type: 1,
                            first_name: "a".into(),
                            skipped: 2,
                            empty: None,
                            when: vec![],
                            inner: Inner::Two { a_b: 1 },
                            more: vec![
                                Inner::One,
                                Inner::Three(3),
                                Inner::Four(Point { x: 0, y: 0 })
                            ],
                        },
                        Renamed {
                            r#type: 1,
                            first_name: "a".into(),
                            skipped: 2,
                            empty: Some(4),
                            when: vec![1],
                            inner: Inner::One,
                            more: vec![],
                        },
                        Internal::A { x: 1, y: None },
                        Internal::B,
                        Internal::C(Point { x: 1, y: 2 }),
                        Ok::<Point, String>(Point { x: 1, y: 2 }),
                        Err::<Point, String>("e".into()),
                    );
                }
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

    /// The corpus types again, with `#[skathi::data]`'s `FromJs`/`IntoJs` where it gives them.
    #[cfg(feature = "types")]
    mod derived {
        use std::collections::BTreeMap;

        use serde_json::Value as Json;

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Point {
            pub x: i32,
            pub y: i32,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Defaults {
            pub x: Option<i32>,
            #[serde(default)]
            pub y: u8,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(rename_all = "camelCase")]
        pub struct Camel {
            pub some_field: i32,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(deny_unknown_fields)]
        pub struct Strict {
            pub x: i32,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Flat {
            pub x: i32,
            #[serde(flatten)]
            pub rest: BTreeMap<String, Json>,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Nested {
            pub x: i32,
            pub y: i32,
            pub n: Option<Box<Nested>>,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub enum Shape {
            Unit,
            Wrap(i32),
            Pair(i32, String),
            Named { x: i32, y: i32 },
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(tag = "tag")]
        pub enum Tagged {
            A { x: i32 },
            B,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(tag = "t", content = "c")]
        pub enum Adjacent {
            A(i32),
            B,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(untagged)]
        pub enum Either {
            N(i32),
            S(String),
            P(Point),
            L(Vec<Either>),
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Newtype(pub i32);

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Pair(pub i32, pub i32);

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Unit;

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(rename_all = "PascalCase")]
        pub struct Renamed {
            pub r#type: i32,
            #[serde(rename = "given")]
            pub first_name: String,
            #[serde(skip)]
            pub skipped: i32,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub empty: Option<i32>,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub when: Vec<u32>,
            #[serde(default = "Inner::fallback")]
            pub inner: Inner,
            pub more: Vec<Inner>,
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(rename_all = "kebab-case")]
        pub enum Inner {
            One,
            #[serde(rename_all = "camelCase")]
            Two {
                a_b: i32,
            },
            #[serde(rename = "three!")]
            Three(i32),
            Four(Point),
        }

        impl Inner {
            fn fallback() -> Self {
                Inner::Three(9)
            }
        }

        const _: fn() = || {
            fn derived<T: super::FromJs + super::IntoJs>() {}
            derived::<Point>();
            derived::<Defaults>();
            derived::<Camel>();
            derived::<Nested>();
            derived::<Shape>();
            derived::<Tagged>();
            derived::<Adjacent>();
            derived::<Either>();
            derived::<Newtype>();
            derived::<Renamed>();
            derived::<Inner>();
            derived::<Internal>();
            derived::<Frame>();
        };

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        #[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
        pub enum Internal {
            A { x: i32, y: Option<i32> },
            B,
            C(Point),
        }

        #[skathi_macros::data(crate = crate::embed)]
        #[derive(Debug, PartialEq)]
        pub struct Frame {
            pub data: Vec<u8>,
            pub mask: Option<Vec<u8>>,
        }
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Frame {
        #[serde(with = "crate::embed::bytes")]
        data: Vec<u8>,
        #[serde(with = "crate::embed::bytes::option", default)]
        mask: Option<Vec<u8>>,
    }

    #[test]
    fn typed_arrays_and_buffers_are_bytes_and_bytes_are_uint8_arrays() {
        use crate::embed::Bytes;
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|runtime| {
                let context = runtime.ctx();
                install(context).unwrap();
                let read = |src: &str| direct::<Bytes>(context, eval(context, src)).map(|b| b.0);
                assert_eq!(read("new Uint8Array([1, 2, 255])"), Ok(vec![1, 2, 255]));
                assert_eq!(
                    read("new Uint8Array([9, 1, 2, 3]).subarray(1, 3)"),
                    Ok(vec![1, 2])
                );
                assert_eq!(read("new Uint8Array([4, 5]).buffer"), Ok(vec![4, 5]));
                assert_eq!(
                    read("new DataView(new Uint8Array([4, 5, 6]).buffer, 1)"),
                    Ok(vec![5, 6])
                );
                assert_eq!(read("new Uint16Array([0x0102])"), Ok(vec![2, 1]));
                assert_eq!(read("[7, 8]"), Ok(vec![7, 8]));
                assert_eq!(read("new Uint8Array(0)"), Ok(vec![]));
                assert_eq!(
                    read("'ab'"),
                    Err("argument 1: invalid type: string \"ab\", expected a Uint8Array, an ArrayBuffer or an array of bytes".into())
                );
                assert_eq!(
                    read("[256]"),
                    Err("argument 1: invalid value: integer `256`, expected u8".into())
                );

                assert_eq!(
                    direct::<Vec<u8>>(context, eval(context, "new Uint8Array([1, 2])")),
                    Ok(vec![1, 2])
                );
                assert_eq!(
                    direct::<Vec<f32>>(context, eval(context, "new Float32Array([0.5, -2])")),
                    Ok(vec![0.5, -2.0])
                );
                assert_eq!(
                    direct::<Vec<i64>>(context, eval(context, "new BigInt64Array([-5n])")),
                    Ok(vec![-5])
                );
                assert_eq!(
                    direct::<(u8, u8)>(context, eval(context, "new Uint8Array([1, 2, 3])")),
                    Err("argument 1: invalid length 3, expected fewer elements in array".into())
                );
                assert_eq!(
                    direct::<Vec<u8>>(context, eval(context, "new Int16Array([-1])")),
                    Err("argument 1: invalid value: integer `-1`, expected u8".into())
                );
                assert_eq!(
                    direct::<Json>(context, eval(context, "new Uint8Array([1, 2])")),
                    Ok(serde_json::json!({ "0": 1, "1": 2 })),
                    "a value of any type reads a typed array as JSON.stringify sees it"
                );
                assert_eq!(
                    direct::<Frame>(context, eval(context, "({ data: new Uint8Array([3]) })")),
                    Ok(Frame {
                        data: vec![3],
                        mask: None
                    })
                );

                let floats = eval(context, "new Float32Array([1.5, 2.5])");
                let data = context.typedarray_info(floats).unwrap().data;
                let walk = Walk::new(context);
                let slice = Slice::<f32>::deserialize(walk.arg(0, floats)).unwrap();
                // SAFETY: `floats` is alive on this frame and no JS runs.
                let read = unsafe { slice.get() };
                assert_eq!(
                    (read, read.as_ptr() as usize),
                    (&[1.5f32, 2.5][..], data as usize)
                );
                let slice =
                    Slice::<f32>::deserialize(walk.arg(0, eval(context, "[1, 2]"))).unwrap();
                assert_eq!(unsafe { slice.get() }, &[1.0f32, 2.0]);
                let wide = eval(context, "new Uint16Array([0x0102])");
                let slice = Slice::<u8>::deserialize(walk.arg(0, wide)).unwrap();
                assert_eq!(unsafe { slice.get() }, &[2u8, 1]);

                let show = context
                    .eval("(v) => `${v.constructor.name}:${Array.from(v.data ?? v).join(',')}`")
                    .unwrap();
                let shown = |v: Value| {
                    context
                        .to_rust_string(context.call(show, Value::undefined(), &[v]).unwrap())
                        .unwrap()
                };
                let bytes = Bytes(vec![1, 2, 255]);
                let direct = bytes
                    .serialize(Out {
                        build: Engine(context),
                        top: false,
                    })
                    .unwrap();
                let tree = owned(&bytes, false).unwrap();
                assert_eq!(shown(direct), "Uint8Array:1,2,255");
                assert_eq!(shown(tree.to_js(context).unwrap()), "Uint8Array:1,2,255");
                let empty = owned(&Bytes(Vec::new()), false).unwrap();
                assert_eq!(shown(empty.to_js(context).unwrap()), "Uint8Array:");
                let frame = Frame {
                    data: vec![4],
                    mask: None,
                };
                let frame = to_js(context, &frame, false).unwrap();
                let data = context.get(frame, &context.atom("data")).unwrap();
                assert_eq!(shown(data), "Uint8Array:4");

                #[cfg(feature = "types")]
                {
                    let read = |src: &str| {
                        read_with(context, eval(context, src), |js| {
                            (&&Pick::<derived::Frame>::new()).read(js)
                        })
                    };
                    assert_eq!(
                        read("({ data: new Uint8Array([3]), mask: new Uint8Array([4]).buffer })"),
                        Ok(derived::Frame {
                            data: vec![3],
                            mask: Some(vec![4])
                        })
                    );
                    assert_eq!(
                        read("({ data: [1, 2], mask: null })"),
                        Ok(derived::Frame {
                            data: vec![1, 2],
                            mask: None
                        })
                    );
                    assert_eq!(
                        read("({ mask: null })"),
                        Err("argument 1: missing field `data`".into())
                    );
                    let frame = derived::Frame {
                        data: vec![5, 6],
                        mask: Some(vec![]),
                    };
                    let built = (&&Pick::of(&frame)).answer(&frame, context).unwrap();
                    let field = |name: &str| context.get(built, &context.atom(name)).unwrap();
                    assert_eq!(shown(field("data")), "Uint8Array:5,6");
                    assert_eq!(shown(field("mask")), "Uint8Array:");
                }
            })
        });
    }
}
