//! Binary data for native modules: a `Uint8Array` in JS. `#[serde(with = "skathi::bytes")]` puts
//! a `Vec<u8>` field in that form; `#[skathi::data]` does it for every `Vec<u8>` field.

use std::fmt;
use std::ops::{Deref, DerefMut};

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// Bytes that reach JS as a `Uint8Array`, read from any `ArrayBufferView`, an `ArrayBuffer` or
/// an array of numbers.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bytes(pub Vec<u8>);

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Deref for Bytes {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl DerefMut for Bytes {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl From<&[u8]> for Bytes {
    fn from(v: &[u8]) -> Self {
        Self(v.to_vec())
    }
}

impl From<Bytes> for Vec<u8> {
    fn from(v: Bytes) -> Self {
        v.0
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_byte_buf(BytesVisitor).map(Bytes)
    }
}

struct BytesVisitor;

impl<'de> Visitor<'de> for BytesVisitor {
    type Value = Vec<u8>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a Uint8Array, an ArrayBuffer or an array of bytes")
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
        Ok(v.to_vec())
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
        Ok(v)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 20));
        while let Some(b) = seq.next_element::<u8>()? {
            out.push(b);
        }
        Ok(out)
    }
}

#[cfg(feature = "types")]
impl specta::Type for Bytes {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("Uint8Array").into()
    }
}

/// `#[serde(with = "skathi::bytes")]` for a `Vec<u8>`.
pub fn serialize<S: Serializer>(v: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_bytes(v)
}

/// `#[serde(with = "skathi::bytes")]` for a `Vec<u8>`.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    deserializer.deserialize_byte_buf(BytesVisitor)
}

/// `#[serde(with = "skathi::bytes::option", default)]` for an `Option<Vec<u8>>`.
pub mod option {
    use super::*;

    struct Slice<'a>(&'a [u8]);

    impl Serialize for Slice<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(self.0)
        }
    }

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => serializer.serialize_some(&Slice(v)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        Ok(Option::<Bytes>::deserialize(deserializer)?.map(|b| b.0))
    }
}
