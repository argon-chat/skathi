//! Passkeys (`navigator.credentials` with `publicKey`): webauthn.dll on Windows, AuthenticationServices on
//! macOS. Binary fields cross as base64url. web/credentials.rs builds the client data; where the system
//! writes its own, the result carries it as `clientDataJSON`.

use std::collections::HashMap;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
pub use mac::{available, cancel, start_create, start_get};
#[cfg(target_os = "windows")]
pub use win::{available, cancel, start_create, start_get};

/// A minimal CBOR reader for the COSE key in authenticator data.
struct Cbor<'a> {
    b: &'a [u8],
    at: usize,
}

enum Item<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    Other,
}

impl<'a> Cbor<'a> {
    fn head(&mut self) -> Option<(u8, u64)> {
        let first = *self.b.get(self.at)?;
        self.at += 1;
        let (major, info) = (first >> 5, first & 0x1f);
        let n = match info {
            0..=23 => info as u64,
            24..=27 => {
                let len = 1usize << (info - 24);
                let raw = self.b.get(self.at..self.at + len)?;
                self.at += len;
                raw.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64)
            }
            _ => return None,
        };
        Some((major, n))
    }

    fn item(&mut self) -> Option<Item<'a>> {
        let (major, n) = self.head()?;
        Some(match major {
            0 => Item::Int(i64::try_from(n).ok()?),
            1 => Item::Int(-1 - i64::try_from(n).ok()?),
            2 | 3 => {
                let end = self.at.checked_add(usize::try_from(n).ok()?)?;
                let raw = self.b.get(self.at..end)?;
                self.at = end;
                if major == 2 {
                    Item::Bytes(raw)
                } else {
                    Item::Other
                }
            }
            4 => {
                for _ in 0..n {
                    self.item()?;
                }
                Item::Other
            }
            5 => {
                for _ in 0..n.checked_mul(2)? {
                    self.item()?;
                }
                Item::Other
            }
            6 => {
                self.item()?;
                Item::Other
            }
            _ => Item::Other,
        })
    }
}

/// The `authData` of an attestation object.
#[cfg(any(target_os = "macos", test))]
fn auth_data_of(attestation: &[u8]) -> Option<&[u8]> {
    let mut r = Cbor { b: attestation, at: 0 };
    let (major, n) = r.head()?;
    if major != 5 {
        return None;
    }
    for _ in 0..n {
        let (major, len) = r.head()?;
        let end = r.at.checked_add(usize::try_from(len).ok()?)?;
        let key = r.b.get(r.at..end)?;
        r.at = end;
        let value = r.item()?;
        if let (3, Item::Bytes(b)) = (major, value) {
            if key == b"authData" {
                return Some(b);
            }
        }
    }
    None
}

fn cose_map(auth: &[u8]) -> Option<HashMap<i64, Item<'_>>> {
    if auth.len() < 55 || auth[32] & 0x40 == 0 {
        return None;
    }
    let id_len = u16::from_be_bytes([auth[53], auth[54]]) as usize;
    let mut r = Cbor { b: auth, at: 55 + id_len };
    let (major, n) = r.head()?;
    if major != 5 {
        return None;
    }
    let mut m = HashMap::new();
    for _ in 0..n {
        let Item::Int(k) = r.item()? else { return None };
        m.insert(k, r.item()?);
    }
    Some(m)
}

fn cose_alg(auth: &[u8]) -> Option<i64> {
    match cose_map(auth)?.get(&3)? {
        Item::Int(a) => Some(*a),
        _ => None,
    }
}

fn der_len(n: usize) -> Vec<u8> {
    match n {
        0..=127 => vec![n as u8],
        128..=255 => vec![0x81, n as u8],
        _ => vec![0x82, (n >> 8) as u8, n as u8],
    }
}

fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(der_len(body.len()));
    out.extend_from_slice(body);
    out
}

fn der_uint(v: &[u8]) -> Vec<u8> {
    let v = &v[v.iter().position(|&b| b != 0).unwrap_or(v.len().saturating_sub(1))..];
    let mut body = Vec::with_capacity(v.len() + 1);
    if v.first().is_some_and(|&b| b & 0x80 != 0) {
        body.push(0);
    }
    body.extend_from_slice(v);
    der(0x02, &body)
}

/// `(alg, SubjectPublicKeyInfo DER)` for ES256, EdDSA (Ed25519) and RS256 keys, as
/// `getPublicKey()` returns them.
fn cose_public_key(auth: &[u8]) -> Option<(i64, Vec<u8>)> {
    let m = cose_map(auth)?;
    let int = |k: i64| match m.get(&k) {
        Some(Item::Int(v)) => Some(*v),
        _ => None,
    };
    let raw = |k: i64| match m.get(&k) {
        Some(Item::Bytes(b)) => Some(*b),
        _ => None,
    };
    let alg = int(3)?;
    let spki = match (int(1)?, int(-1)) {
        (2, Some(1)) => {
            let (x, y) = (raw(-2)?, raw(-3)?);
            if x.len() != 32 || y.len() != 32 {
                return None;
            }
            let mut point = vec![0x04];
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            let mut out = hex("3059301306072a8648ce3d020106082a8648ce3d030107034200");
            out.extend(point);
            out
        }
        (1, Some(6)) => {
            let x = raw(-2)?;
            if x.len() != 32 {
                return None;
            }
            let mut out = hex("302a300506032b6570032100");
            out.extend_from_slice(x);
            out
        }
        (3, _) => {
            let (n, e) = (raw(-1)?, raw(-2)?);
            let mut key = der_uint(n);
            key.extend(der_uint(e));
            let mut bits = vec![0];
            bits.extend(der(0x30, &key));
            let mut body = hex("300d06092a864886f70d0101010500");
            body.extend(der(0x03, &bits));
            der(0x30, &body)
        }
        _ => return None,
    };
    Some((alg, spki))
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_with_key(key: &[u8]) -> Vec<u8> {
        let mut a = vec![0u8; 32];
        a.push(0x45);
        a.extend([0, 0, 0, 1]);
        a.extend([0u8; 16]);
        a.extend([0, 2, 0xAA, 0xBB]);
        a.extend_from_slice(key);
        a
    }

    #[test]
    fn an_es256_key_becomes_its_spki() {
        let mut key = vec![0xA5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        key.extend([1u8; 32]);
        key.extend([0x22, 0x58, 0x20]);
        key.extend([2u8; 32]);
        let (alg, spki) = cose_public_key(&auth_with_key(&key)).unwrap();
        assert_eq!(alg, -7);
        assert_eq!(spki.len(), 91);
        assert_eq!(&spki[..27], &hex("3059301306072a8648ce3d020106082a8648ce3d03010703420004")[..]);
        assert_eq!(&spki[27..59], &[1u8; 32]);
    }

    #[test]
    fn an_rs256_key_becomes_its_spki() {
        let mut key = vec![0xA4, 0x01, 0x03, 0x03, 0x39, 0x01, 0x00, 0x20, 0x59, 0x01, 0x00];
        key.extend([0x80u8; 256]);
        key.extend([0x21, 0x43, 0x01, 0x00, 0x01]);
        let (alg, spki) = cose_public_key(&auth_with_key(&key)).unwrap();
        assert_eq!(alg, -257);
        assert_eq!(&spki[..4], &[0x30, 0x82, 0x01, 0x22]);
        assert_eq!(spki.len(), 294);
        assert_eq!(&spki[spki.len() - 5..], &[0x02, 0x03, 0x01, 0x00, 0x01]);
    }

    #[test]
    fn no_attested_data_is_no_key() {
        let mut a = auth_with_key(&[0xA0]);
        a[32] = 0x05;
        assert!(cose_public_key(&a).is_none());
        assert!(cose_public_key(&[0u8; 10]).is_none());
    }

    #[test]
    fn an_attestation_object_yields_its_authenticator_data() {
        let mut att = vec![0xA3, 0x63];
        att.extend(b"fmt");
        att.extend([0x64]);
        att.extend(b"none");
        att.extend([0x67]);
        att.extend(b"attStmt");
        att.extend([0xA0, 0x68]);
        att.extend(b"authData");
        att.extend([0x43, 1, 2, 3]);
        assert_eq!(auth_data_of(&att), Some(&[1u8, 2, 3][..]));
        assert_eq!(auth_data_of(&[0xA0]), None);
        assert_eq!(auth_data_of(&att[..att.len() - 1]), None);
    }
}
