use super::format::truncated;
use super::{BankCodec, BankError};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::io::{self, Read};
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub(super) struct Encoded<'a> {
    pub bytes: &'a [u8],
    pub codec: BankCodec,
    pub chunked: bool,
    pub scramble: Option<([u8; 16], u64)>,
}

impl<'a> Encoded<'a> {
    fn key(&self) -> Option<[u8; 32]> {
        self.scramble.map(|(id, crc)| {
            Sha256::new()
                .chain_update(id)
                .chain_update(crc.to_le_bytes())
                .finalize()
                .into()
        })
    }

    pub(super) fn read(&self, max: usize) -> Result<Cow<'a, [u8]>, BankError> {
        if self.codec == BankCodec::Store {
            if self.bytes.len() > max {
                return Err(too_large());
            }
            return self.stored_range(0..self.bytes.len());
        }
        let key = self.key();
        if self.chunked {
            let chunks = Chunks::parse(self.bytes, key)?;
            if chunks.raw_length > max {
                return Err(too_large());
            }
            return chunks
                .read_range(self.codec, 0..chunks.raw_length)
                .map(Cow::Owned);
        }
        let raw = self.decoder(key, 0..self.bytes.len())?;
        let mut data = Vec::new();
        append_limited(raw, &mut data, max)?;
        Ok(Cow::Owned(data))
    }

    pub(super) fn read_range(&self, range: Range<usize>) -> Result<Cow<'a, [u8]>, BankError> {
        if range.start > range.end {
            return Err(bad_range());
        }
        if self.codec == BankCodec::Store {
            return self.stored_range(range);
        }
        let key = self.key();
        if self.chunked {
            return Chunks::parse(self.bytes, key)?
                .read_range(self.codec, range)
                .map(Cow::Owned);
        }
        let raw = self.decoder(key, 0..self.bytes.len())?;
        let mut out = Vec::new();
        append_range(raw, range, &mut out, false)?;
        Ok(Cow::Owned(out))
    }

    fn stored_range(&self, range: Range<usize>) -> Result<Cow<'a, [u8]>, BankError> {
        let bytes = self.bytes.get(range.clone()).ok_or_else(bad_range)?;
        match self.key() {
            None => Ok(Cow::Borrowed(bytes)),
            Some(key) => {
                let mut raw = Vec::new();
                raw.try_reserve_exact(bytes.len())
                    .map_err(|_| too_large())?;
                raw.extend_from_slice(bytes);
                apply_key(&mut raw, range.start, &key);
                Ok(Cow::Owned(raw))
            }
        }
    }

    fn decoder(
        &self,
        key: Option<[u8; 32]>,
        range: Range<usize>,
    ) -> Result<Box<dyn Read + 'a>, BankError> {
        open_decoder(self.codec, self.bytes, key, range)
    }
}

fn too_large() -> BankError {
    BankError::invalid("decoded data exceeds the requested limit or available memory")
}
fn bad_range() -> BankError {
    BankError::invalid("requested range outside entry")
}

fn apply_key(bytes: &mut [u8], offset: usize, key: &[u8; 32]) {
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte ^= key[(offset + i) % key.len()];
    }
}

struct Input<'a> {
    bytes: &'a [u8],
    offset: usize,
    key: Option<[u8; 32]>,
}

impl Read for Input<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.bytes.read(buf)?;
        if let Some(key) = &self.key {
            apply_key(&mut buf[..n], self.offset, key);
        }
        self.offset += n;
        Ok(n)
    }
}

fn open_decoder<'a>(
    codec: BankCodec,
    bytes: &'a [u8],
    key: Option<[u8; 32]>,
    range: Range<usize>,
) -> Result<Box<dyn Read + 'a>, BankError> {
    let source = Input {
        bytes: &bytes[range.clone()],
        offset: range.start,
        key,
    };
    match codec {
        BankCodec::Brotli => {
            use brotli_decompressor::BrotliDecoderParameter::BROTLI_DECODER_PARAM_LARGE_WINDOW;
            let mut decoder = brotli_decompressor::Decompressor::new(source, 8192);
            // DeHive's .NET BrotliStream uses the standard window (RFC 7932).
            decoder.set_parameter(BROTLI_DECODER_PARAM_LARGE_WINDOW, 0);
            Ok(Box::new(decoder))
        }
        BankCodec::Zstd => Ok(Box::new(zstd::stream::read::Decoder::new(source)?)),
        BankCodec::Store => Ok(Box::new(source)),
    }
}

fn append_limited(
    mut reader: impl Read,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<usize, BankError> {
    let mut buf = [0; 8192];
    let mut total = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(total);
        }
        if n > limit - total {
            return Err(too_large());
        }
        out.try_reserve(n).map_err(|_| too_large())?;
        out.extend_from_slice(&buf[..n]);
        total += n;
    }
}

// Skip in the decoder's stream so a one-byte range never allocates a whole chunk.
fn append_range(
    mut reader: impl Read,
    range: Range<usize>,
    out: &mut Vec<u8>,
    expect_end: bool,
) -> Result<(), BankError> {
    let skip = range.start as u64;
    if io::copy(&mut reader.by_ref().take(skip), &mut io::sink())? != skip {
        return Err(bad_range());
    }
    let length = range.len();
    if append_limited(reader.by_ref().take(length as u64), out, length)? != length {
        return Err(bad_range());
    }
    if expect_end && reader.read(&mut [0u8])? != 0 {
        return Err(BankError::invalid("chunk exceeds its declared size"));
    }
    Ok(())
}

struct Chunks<'a> {
    bytes: &'a [u8],
    key: Option<[u8; 32]>,
    footer: usize,
    raw_length: usize,
    size: usize,
    count: usize,
}

fn number<const N: usize>(
    bytes: &[u8],
    pos: usize,
    key: Option<&[u8; 32]>,
) -> Result<[u8; N], BankError> {
    let mut value: [u8; N] = bytes
        .get(pos..)
        .and_then(|s| s.get(..N))
        .ok_or_else(truncated)?
        .try_into()
        .unwrap();
    if let Some(key) = key {
        apply_key(&mut value, pos, key);
    }
    Ok(value)
}

impl<'a> Chunks<'a> {
    fn parse(bytes: &'a [u8], key: Option<[u8; 32]>) -> Result<Self, BankError> {
        let tail = bytes.len().checked_sub(4).ok_or_else(truncated)?;
        let footer_len = u32::from_le_bytes(number(bytes, tail, key.as_ref())?) as usize;
        if footer_len < 16 {
            return Err(BankError::invalid("short chunk footer"));
        }
        let footer = tail.checked_sub(footer_len).ok_or_else(truncated)?;
        let raw_length = usize::try_from(u64::from_le_bytes(number(bytes, footer, key.as_ref())?))
            .map_err(|_| too_large())?;
        let size = u32::from_le_bytes(number(bytes, footer + 8, key.as_ref())?) as usize;
        let count = u32::from_le_bytes(number(bytes, footer + 12, key.as_ref())?) as usize;
        if size == 0
            || raw_length.div_ceil(size) != count
            || count.checked_mul(4) != Some(footer_len - 16)
        {
            return Err(BankError::invalid("invalid chunk table dimensions"));
        }
        Ok(Self {
            bytes,
            key,
            footer,
            raw_length,
            size,
            count,
        })
    }

    fn read_range(&self, codec: BankCodec, range: Range<usize>) -> Result<Vec<u8>, BankError> {
        if range.end > self.raw_length {
            return Err(bad_range());
        }
        let mut out = Vec::new();
        let mut compressed_start = 0usize;
        for index in 0..self.count {
            let len_pos = self.footer + 16 + index * 4;
            let len = u32::from_le_bytes(number(self.bytes, len_pos, self.key.as_ref())?) as usize;
            let end = compressed_start
                .checked_add(len)
                .filter(|&e| len > 0 && e <= self.footer)
                .ok_or_else(|| BankError::invalid("invalid compressed chunk length"))?;
            let raw_start = index * self.size;
            let raw_end = raw_start.saturating_add(self.size).min(self.raw_length);
            let from = raw_start.max(range.start);
            let to = raw_end.min(range.end);
            if from < to {
                let raw = open_decoder(codec, self.bytes, self.key, compressed_start..end)?;
                append_range(
                    raw,
                    from - raw_start..to - raw_start,
                    &mut out,
                    to == raw_end,
                )?;
            }
            compressed_start = end;
        }
        if compressed_start != self.footer {
            return Err(BankError::invalid("chunk lengths do not cover the data"));
        }
        Ok(out)
    }
}
