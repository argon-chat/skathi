//! Types that read themselves from engine values and write themselves into a result without
//! serde. `#[skathi::data]` derives this; the std types forward it to what they hold, and a type
//! without it, such as a plain serde type, is read and written through serde instead.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::hash::{BuildHasher, Hash};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

use redjs::glue;
use serde::de::{self, DeserializeOwned, Visitor};
use serde::Serialize;

use super::{atom, Error, Js, Key, Node, Out, OutError, Owned, Tree};
use crate::js::{Context, Value};

/// Read straight from engine values, with the meaning a JSON round trip gives them.
/// `#[skathi::data]` derives it.
pub trait FromJs: DeserializeOwned {
    #[doc(hidden)]
    fn from_js(js: Js<'_, '_>) -> Result<Self, Error> {
        Self::deserialize(js)
    }
}

/// Written straight into a result, as JSON.parse would make it of serde_json's text.
/// `#[skathi::data]` derives it.
pub trait IntoJs: Serialize {
    #[doc(hidden)]
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        self.serialize(Out {
            build: out,
            top: false,
        })
    }

    /// What a call that returned `self` gives the app.
    #[doc(hidden)]
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        super::to_js(context, self, true)
    }

    /// The same, for a task: made on its worker, built on the JS thread.
    #[doc(hidden)]
    fn answer_owned(&self) -> Result<Owned, OutError> {
        super::owned(self, true)
    }
}

/// `answer` of a value that is always an object or an array.
#[doc(hidden)]
pub fn answer_compound<T: IntoJs + ?Sized>(
    value: &T,
    context: &Context,
) -> Result<Value, OutError> {
    super::build_with(context, |out| value.encode(out))
}

/// `answer_owned` of a value that is always an object or an array.
#[doc(hidden)]
pub fn owned_compound<T: IntoJs + ?Sized>(value: &T) -> Result<Owned, OutError> {
    super::encode(|out| value.encode(out))
}

macro_rules! plain {
    ($($t:ty),* $(,)?) => {
        $(impl FromJs for $t {}
        impl IntoJs for $t {})*
    };
}

plain! {
    bool, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, char,
    String, (), serde_json::Value, crate::embed::Bytes, crate::embed::NativeBuffer,
}

impl FromJs for crate::embed::HostFile {}

impl IntoJs for str {}

impl<T: IntoJs + ?Sized> IntoJs for &T {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        (**self).encode(out)
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        (**self).answer(context)
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        (**self).answer_owned()
    }
}

impl<T: FromJs> FromJs for Box<T> {
    fn from_js(js: Js<'_, '_>) -> Result<Self, Error> {
        T::from_js(js).map(Box::new)
    }
}

impl<T: IntoJs + ?Sized> IntoJs for Box<T> {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        (**self).encode(out)
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        (**self).answer(context)
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        (**self).answer_owned()
    }
}

impl<T: FromJs> FromJs for Option<T> {
    fn from_js(js: Js<'_, '_>) -> Result<Self, Error> {
        match js.node()? {
            None | Some(Node::Null) => Ok(None),
            Some(node) => {
                js.pre.set(Some(node));
                T::from_js(js).map(Some)
            }
        }
    }
}

impl<T: IntoJs> IntoJs for Option<T> {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        match self {
            Some(v) => v.encode(out),
            None => {
                out.null_value();
                Ok(())
            }
        }
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        match self {
            Some(v) => v.answer(context),
            None => Ok(Value::null()),
        }
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        match self {
            Some(v) => v.answer_owned(),
            None => super::owned(&(), true),
        }
    }
}

impl<T: FromJs> FromJs for Vec<T> {
    fn from_js(js: Js<'_, '_>) -> Result<Self, Error> {
        let node: Node<'static> = js.require()?;
        let Node::Array(array) = node else {
            js.pre.set(Some(node));
            return <Self as serde::Deserialize>::deserialize(js);
        };
        js.items(array, T::from_js)
    }
}

impl<T: IntoJs> IntoJs for Vec<T> {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        self.as_slice().encode(out)
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        answer_compound(self, context)
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        owned_compound(self)
    }
}

impl<T: IntoJs> IntoJs for [T] {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        let open = out.begin_array();
        for v in self {
            v.encode(out)?;
        }
        out.end(open, self.len() as u32);
        Ok(())
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        answer_compound(self, context)
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        owned_compound(self)
    }
}

impl<V: FromJs, S: BuildHasher + Default> FromJs for HashMap<String, V, S> {
    fn from_js(js: Js<'_, '_>) -> Result<Self, Error> {
        let mut map = HashMap::default();
        match js.entries(|key, value| {
            map.insert(key, V::from_js(value)?);
            Ok(())
        })? {
            true => Ok(map),
            false => <Self as serde::Deserialize>::deserialize(js),
        }
    }
}

impl<V: FromJs> FromJs for BTreeMap<String, V> {
    fn from_js(js: Js<'_, '_>) -> Result<Self, Error> {
        let mut map = BTreeMap::new();
        match js.entries(|key, value| {
            map.insert(key, V::from_js(value)?);
            Ok(())
        })? {
            true => Ok(map),
            false => <Self as serde::Deserialize>::deserialize(js),
        }
    }
}

fn encode_entries<'v, K: AsRef<str> + 'v, V: IntoJs + 'v>(
    entries: impl ExactSizeIterator<Item = (&'v K, &'v V)>,
    out: Tree<'_>,
) -> Result<(), OutError> {
    let open = out.begin_object();
    let count = entries.len() as u32;
    for (k, v) in entries {
        out.entry_key(k.as_ref());
        v.encode(out)?;
    }
    out.end(open, count);
    Ok(())
}

impl<K: AsRef<str> + Eq + Hash + Serialize, V: IntoJs, S: BuildHasher> IntoJs for HashMap<K, V, S> {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        encode_entries(self.iter(), out)
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        answer_compound(self, context)
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        owned_compound(self)
    }
}

impl<K: AsRef<str> + Ord + Serialize, V: IntoJs> IntoJs for BTreeMap<K, V> {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        encode_entries(self.iter(), out)
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        answer_compound(self, context)
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        owned_compound(self)
    }
}

impl<T: IntoJs, E: Serialize> IntoJs for Result<T, E> {
    fn encode(&self, out: Tree<'_>) -> Result<(), OutError> {
        let open = out.begin_object();
        match self {
            Ok(v) => {
                out.key("Ok");
                v.encode(out)?;
            }
            Err(e) => {
                out.key("Err");
                e.serialize(Out {
                    build: out,
                    top: false,
                })?;
            }
        }
        out.end(open, 1);
        Ok(())
    }
    fn answer(&self, context: &Context) -> Result<Value, OutError> {
        match self {
            Ok(v) => v.answer(context),
            Err(e) => Err(error(e)),
        }
    }
    fn answer_owned(&self) -> Result<Owned, OutError> {
        match self {
            Ok(v) => v.answer_owned(),
            Err(e) => Err(error(e)),
        }
    }
}

/// A call's `Err(e)`, as serde_json writes `e`.
fn error<E: Serialize>(e: &E) -> OutError {
    match serde_json::to_string(e) {
        Ok(json) => OutError::Err(json),
        Err(e) => OutError::Message(e.to_string()),
    }
}

/// The JS names of a `#[skathi::data]` type's fields, made into atoms once per realm.
#[doc(hidden)]
pub struct Keys {
    names: &'static [&'static str],
    slot: AtomicUsize,
}

static NEXT_SLOT: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    /// Per realm, the atoms of every `Keys` by slot; an atom is never freed.
    static TABLES: RefCell<(usize, Vec<Option<Box<[*mut c_void]>>>)> =
        const { RefCell::new((0, Vec::new())) };
}

impl Keys {
    pub const fn new(names: &'static [&'static str]) -> Self {
        Self {
            names,
            slot: AtomicUsize::new(0),
        }
    }

    fn slot(&self) -> usize {
        let slot = self.slot.load(Ordering::Relaxed);
        if slot != 0 {
            return slot - 1;
        }
        let fresh = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
        match self
            .slot
            .compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => fresh - 1,
            Err(taken) => taken - 1,
        }
    }

    fn atoms(&self, context: &Context) -> *const *mut c_void {
        let slot = self.slot();
        TABLES.with(|tables| {
            let mut tables = tables.borrow_mut();
            let realm = context.raw() as usize;
            if tables.0 != realm {
                *tables = (realm, Vec::new());
            }
            if tables.1.len() <= slot {
                tables.1.resize_with(slot + 1, || None);
            }
            tables.1[slot]
                .get_or_insert_with(|| self.names.iter().map(|n| atom(context, n)).collect())
                .as_ptr()
        })
    }
}

/// The values of a struct's fields, read in one engine call.
#[doc(hidden)]
pub struct Fields<'p, 'w, 'a, const N: usize> {
    parent: &'p Js<'w, 'a>,
    values: [u64; N],
    nodes: [glue::RjsJsonNode; N],
    atoms: *const *mut c_void,
}

impl<'w, 'a, const N: usize> Fields<'_, 'w, 'a, N> {
    /// Field `i`, `None` when JSON would leave it out.
    #[inline]
    pub fn get<T>(
        &self,
        i: usize,
        read: impl FnOnce(Js<'w, 'a>) -> Result<T, Error>,
    ) -> Result<Option<T>, Error> {
        let v = self.values[i];
        if v == 0 {
            return Ok(None);
        }
        // SAFETY: `atoms` holds N atoms, alive for the realm.
        let atom = unsafe { *self.atoms.add(i) };
        let child = self.parent.child(Key::Atom(atom), Value(v));
        let raw = &self.nodes[i];
        if raw.kind != glue::json::UNREAD {
            // SAFETY: the string is `v`, held in `values` on this stack while it is read.
            if let Some(node) = unsafe { super::made(self.parent.context(), raw, false) }? {
                child.pre.set(Some(node));
                return read(child).map(Some);
            }
        }
        match child.node()? {
            None => Ok(None),
            Some(node) => {
                child.pre.set(Some(node));
                read(child).map(Some)
            }
        }
    }
}

/// A struct's fields read by name from an object; `None` for anything else (an array, a Proxy,
/// a string), which the caller hands to serde with the node it read.
#[doc(hidden)]
pub fn read_struct<'w, 'a, T, const N: usize>(
    js: &Js<'w, 'a>,
    keys: &'static Keys,
    fields: impl FnOnce(&Fields<'_, 'w, 'a, N>) -> Result<T, Error>,
) -> Result<Option<T>, Error> {
    let node: Node<'static> = js.require()?;
    let Node::Object(object) = node else {
        js.pre.set(Some(node));
        return Ok(None);
    };
    let context = js.context();
    let _entered = js.walk.enter(object)?;
    let atoms = keys.atoms(context);
    let mut values = [0u64; N];
    let mut nodes = [glue::RjsJsonNode::default(); N];
    // SAFETY: plain FFI on a live object; `atoms` holds N atoms, `values` and `nodes` N slots.
    match unsafe {
        glue::rjs_json_fields(
            context.raw(),
            object.0,
            atoms,
            N,
            values.as_mut_ptr(),
            nodes.as_mut_ptr(),
        )
    } {
        glue::OK => {}
        glue::EXCEPTION => return Err(Error::Thrown(Value(values[0]))),
        _ => {
            js.pre.set(Some(Node::Object(object)));
            return Ok(None);
        }
    }
    fields(&Fields {
        parent: js,
        values,
        nodes,
        atoms,
    })
    .map(Some)
}

/// A unit variant named by a string; `None` for anything else, left for serde.
#[doc(hidden)]
pub fn unit_variant<T>(
    js: &Js<'_, '_>,
    pick: impl FnOnce(&str) -> Option<T>,
) -> Result<Option<T>, Error> {
    let node: Node<'static> = js.require()?;
    if let Node::Str(name) = &node {
        if let Some(v) = pick(name) {
            return Ok(Some(v));
        }
    }
    js.pre.set(Some(node));
    Ok(None)
}

/// A field the object does not have: `None` for an `Option`, else serde's "missing field".
#[doc(hidden)]
pub fn missing<T: DeserializeOwned>(field: &'static str) -> Result<T, Error> {
    T::deserialize(Missing(field))
}

struct Missing(&'static str);

impl<'de> de::Deserializer<'de> for Missing {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
        Err(de::Error::missing_field(self.0))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_none()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf unit
        unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier ignored_any
    }
}

/// An object or array being written; [`Tree::end`] closes it with its count.
#[doc(hidden)]
pub struct Open(usize);

impl Tree<'_> {
    pub fn begin_object(self) -> Open {
        Open(self.open(super::OBJECT).0)
    }

    pub fn begin_array(self) -> Open {
        Open(self.open(super::ARRAY).0)
    }

    pub fn end(self, open: Open, count: u32) {
        self.close((open.0, count));
    }

    /// A field name; its value comes next.
    pub fn key(self, name: &'static str) {
        super::Build::field(self, &mut (0, 0), name);
    }

    fn entry_key(self, name: &str) {
        self.text(super::ENTRY, name);
    }

    pub fn str_value(self, v: &str) {
        self.text(super::STRING, v);
    }

    pub fn null_value(self) {
        self.tag(super::NULL);
    }
}

/// Bytes as a `Uint8Array`, or null.
#[doc(hidden)]
pub fn encode_bytes(v: Option<&[u8]>, out: Tree<'_>) {
    match v {
        Some(v) => {
            let _ = super::Build::bytes(out, v);
        }
        None => out.null_value(),
    }
}

/// `v` written through serde.
#[doc(hidden)]
pub fn encode_serde<T: Serialize + ?Sized>(v: &T, out: Tree<'_>) -> Result<(), OutError> {
    v.serialize(Out {
        build: out,
        top: false,
    })
}

/// Picks the direct way for a type that has it and serde for one that has not:
/// `(&&Pick::<T>::new()).read(js)` finds [`ReadDirect`] on `&Pick<T>` before [`ReadSerde`] on
/// `Pick<T>`.
#[doc(hidden)]
pub struct Pick<T: ?Sized>(PhantomData<fn() -> *const T>);

impl<T: ?Sized> Pick<T> {
    pub const fn new() -> Self {
        Pick(PhantomData)
    }

    pub fn of(_: &T) -> Self {
        Pick(PhantomData)
    }
}

#[doc(hidden)]
pub trait ReadDirect<T> {
    fn read(&self, js: Js<'_, '_>) -> Result<T, Error>;
}

impl<T: FromJs> ReadDirect<T> for &Pick<T> {
    #[inline]
    fn read(&self, js: Js<'_, '_>) -> Result<T, Error> {
        T::from_js(js)
    }
}

#[doc(hidden)]
pub trait ReadSerde<T> {
    fn read(&self, js: Js<'_, '_>) -> Result<T, Error>;
}

impl<T: DeserializeOwned> ReadSerde<T> for Pick<T> {
    #[inline]
    fn read(&self, js: Js<'_, '_>) -> Result<T, Error> {
        T::deserialize(js)
    }
}

#[doc(hidden)]
pub trait WriteDirect<T: ?Sized> {
    fn encode(&self, v: &T, out: Tree<'_>) -> Result<(), OutError>;
    fn answer(&self, v: &T, context: &Context) -> Result<Value, OutError>;
    fn answer_owned(&self, v: &T) -> Result<Owned, OutError>;
}

impl<T: IntoJs + ?Sized> WriteDirect<T> for &Pick<T> {
    #[inline]
    fn encode(&self, v: &T, out: Tree<'_>) -> Result<(), OutError> {
        v.encode(out)
    }
    #[inline]
    fn answer(&self, v: &T, context: &Context) -> Result<Value, OutError> {
        v.answer(context)
    }
    fn answer_owned(&self, v: &T) -> Result<Owned, OutError> {
        v.answer_owned()
    }
}

#[doc(hidden)]
pub trait WriteSerde<T: ?Sized> {
    fn encode(&self, v: &T, out: Tree<'_>) -> Result<(), OutError>;
    fn answer(&self, v: &T, context: &Context) -> Result<Value, OutError>;
    fn answer_owned(&self, v: &T) -> Result<Owned, OutError>;
}

impl<T: Serialize + ?Sized> WriteSerde<T> for Pick<T> {
    fn encode(&self, v: &T, out: Tree<'_>) -> Result<(), OutError> {
        encode_serde(v, out)
    }
    fn answer(&self, v: &T, context: &Context) -> Result<Value, OutError> {
        super::to_js(context, v, true)
    }
    fn answer_owned(&self, v: &T) -> Result<Owned, OutError> {
        super::owned(v, true)
    }
}
