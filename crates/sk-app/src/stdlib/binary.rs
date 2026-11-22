//! `skathi:binary`: cursors over bytes, bit streams, byte order, LEB128 and CRCs.

use std::ffi::c_void;
use std::sync::OnceLock;

use encoding_rs::Encoding;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};

const SAFE: f64 = 9_007_199_254_740_992.0;
const NATIVE_LITTLE: bool = cfg!(target_endian = "little");
/// The `NativeBuffer` a reader is over, checked before every read.
const SOURCE: &str = "skathi.source";

fn host_class<T>(cell: &OnceLock<usize>, name: &str) -> JSClassRef {
    *cell.get_or_init(|| unsafe { make_host_class::<T>(name) } as usize) as JSClassRef
}

fn uint(v: u32) -> u64 {
    if v <= i32::MAX as u32 {
        enc_i32(v as i32)
    } else {
        enc_f64(v as f64)
    }
}

fn wrap(n: f64) -> i64 {
    if n.is_finite() {
        n.trunc() as i64
    } else {
        0
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Scalar {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

const NUMBER_TYPES: &[&str] = &["u8", "i8", "u16", "i16", "u32", "i32", "f32", "f64"];
const NUMBER_SCALARS: &[Scalar] = &[
    Scalar::U8,
    Scalar::I8,
    Scalar::U16,
    Scalar::I16,
    Scalar::U32,
    Scalar::I32,
    Scalar::F32,
    Scalar::F64,
];

impl Scalar {
    fn parse(name: &str) -> Option<Scalar> {
        Some(match name {
            "u8" => Scalar::U8,
            "i8" => Scalar::I8,
            "u16" => Scalar::U16,
            "i16" => Scalar::I16,
            "u32" => Scalar::U32,
            "i32" => Scalar::I32,
            "u64" => Scalar::U64,
            "i64" => Scalar::I64,
            "f32" => Scalar::F32,
            "f64" => Scalar::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Scalar::U8 | Scalar::I8 => 1,
            Scalar::U16 | Scalar::I16 => 2,
            Scalar::U32 | Scalar::I32 | Scalar::F32 => 4,
            Scalar::U64 | Scalar::I64 | Scalar::F64 => 8,
        }
    }

    fn wide(self) -> bool {
        matches!(self, Scalar::U64 | Scalar::I64)
    }

    /// The bits of `bytes` in the low end of a u64, whatever the byte order.
    fn load(self, bytes: &[u8], little: bool) -> u64 {
        let mut raw = [0u8; 8];
        let n = self.size();
        if little {
            raw[..n].copy_from_slice(&bytes[..n]);
        } else {
            for i in 0..n {
                raw[i] = bytes[n - 1 - i];
            }
        }
        u64::from_le_bytes(raw)
    }

    fn store(self, bits: u64, little: bool, out: &mut Vec<u8>) {
        let raw = bits.to_le_bytes();
        let n = self.size();
        if little {
            out.extend_from_slice(&raw[..n]);
        } else {
            out.extend(raw[..n].iter().rev());
        }
    }

    unsafe fn to_js(self, g: *mut c_void, bits: u64) -> Js<u64> {
        Ok(match self {
            Scalar::U8 => enc_i32(bits as u8 as i32),
            Scalar::I8 => enc_i32(bits as u8 as i8 as i32),
            Scalar::U16 => enc_i32(bits as u16 as i32),
            Scalar::I16 => enc_i32(bits as u16 as i16 as i32),
            Scalar::U32 => uint(bits as u32),
            Scalar::I32 => enc_i32(bits as u32 as i32),
            Scalar::U64 => big_uint(g, bits)?,
            Scalar::I64 => big_int(g, bits as i64)?,
            Scalar::F32 => enc_f64(f32::from_bits(bits as u32) as f64),
            Scalar::F64 => enc_f64(f64::from_bits(bits)),
        })
    }

    fn bits_of(self, n: f64) -> u64 {
        match self {
            Scalar::F32 => (n as f32).to_bits() as u64,
            Scalar::F64 => n.to_bits(),
            _ => wrap(n) as u64,
        }
    }
}

/// The numbers of an array or of a typed array that holds numbers.
pub(super) unsafe fn numbers(g: *mut c_void, v: u64, what: &str) -> Js<Vec<f64>> {
    if let Some(view) = typed_view(g, v) {
        let bytes = view.bytes();
        macro_rules! each {
            ($t:ty) => {
                bytes
                    .chunks_exact(std::mem::size_of::<$t>())
                    .map(|c| <$t>::from_ne_bytes(c.try_into().unwrap()) as f64)
                    .collect()
            };
        }
        let out: Option<Vec<f64>> = match view.kind {
            1 => Some(each!(i8)),
            2 | 3 => Some(each!(u8)),
            4 => Some(each!(i16)),
            5 => Some(each!(u16)),
            6 => Some(each!(i32)),
            7 => Some(each!(u32)),
            8 => Some(each!(f32)),
            9 => Some(each!(f64)),
            _ => None,
        };
        if let Some(out) = out {
            return Ok(out);
        }
    }
    if !is_cell(v) {
        return type_error(g, &format!("{what} must be an array of numbers"));
    }
    let length = count(g, get(g, v, "length"), &format!("the length of {what}"))?;
    let mut out = Vec::with_capacity(length.min(1 << 20));
    for i in 0..length {
        out.push(number(g, get_i(g, v, i as u32))?);
    }
    Ok(out)
}

/// The fields of a struct layout, in the object's own key order.
unsafe fn layout(g: *mut c_void, v: u64, what: &str) -> Js<Vec<(String, Scalar)>> {
    if !is_cell(v) {
        return type_error(
            g,
            &format!("{what}: the layout must be an object of field types"),
        );
    }
    let mut keys = UNDEF;
    if glue::rjs_own_keys(g, v, &mut keys) != 0 {
        return Err(Thrown);
    }
    let mut fields = Vec::new();
    for i in 0..array_len(g, keys) {
        let Some(key) = read_string(g, get_i(g, keys, i)) else {
            continue;
        };
        let name = read_string(g, get(g, v, &key)).unwrap_or_default();
        let Some(scalar) = Scalar::parse(&name) else {
            return type_error(
                g,
                &format!(
                    "{what}: field \"{key}\" has the type \"{name}\"; use u8, i8, u16, i16, u32, i32, u64, i64, f32 or f64"
                ),
            );
        };
        fields.push((key, scalar));
    }
    Ok(fields)
}

struct Reader {
    position: usize,
    little: bool,
}

static READER: OnceLock<usize> = OnceLock::new();

unsafe fn reader<'a>(g: *mut c_void, this: u64) -> Js<(&'a mut Reader, u64, View)> {
    let Some(state) = host_data::<Reader>(g, host_class::<Reader>(&READER, "BinaryReader"), this)
    else {
        return type_error(g, "this is not a BinaryReader");
    };
    let data_view = get(g, this, "view");
    let native = slot_get(g, this, SOURCE);
    if native != UNDEF {
        super::memory::check_current(g, native, get(g, data_view, "buffer"), &"BinaryReader")?;
    }
    match typed_view(g, data_view) {
        Some(view) => Ok((state, data_view, view)),
        None => Err(Thrown),
    }
}

/// The next `n` bytes, with the cursor moved past them.
unsafe fn take<'a>(g: *mut c_void, this: u64, n: usize, what: &str) -> Js<(&'a [u8], bool)> {
    let (state, _, view) = reader(g, this)?;
    let remaining = view.len.saturating_sub(state.position);
    if n > remaining {
        return range_error(
            g,
            &format!("BinaryReader.{what}: {n} bytes wanted, {remaining} remain"),
        );
    }
    let bytes = &view.bytes()[state.position..state.position + n];
    state.position += n;
    Ok((bytes, state.little))
}

std_fn!(fn reader_new(g, args) {
    let source = args.arg(0).0;
    let global = glue::rjs_global(g);
    let data_view = get(g, global, "DataView");
    let mut native = UNDEF;
    let view = if let Some(view) = typed_view(g, source) {
        let offset = enc_f64(view.byte_offset as f64);
        let length = enc_f64(view.len as f64);
        construct(g, data_view, &[get(g, source, "buffer"), offset, length])?
    } else if buffer_source(g, source).is_some() {
        construct(g, data_view, &[source])?
    } else if let Some(buffer) = super::memory::buffer_of(g, source, &"BinaryReader: the source")? {
        native = source;
        construct(g, data_view, &[buffer])?
    } else {
        return type_error(g, "BinaryReader: the source must be an ArrayBuffer, a typed array, a DataView or a NativeBuffer");
    };
    let endian = option(g, args.arg(1).0, "endian");
    let little = choice(g, endian, &["little", "big"], 0, "BinaryReader: endian")? == 0;
    let class = host_class::<Reader>(&READER, "BinaryReader");
    let object = host_create(g, class, prototype(g, "BinaryReader"), Reader { position: 0, little });
    define(g, object, "view", view, 1 | 4);
    if native != UNDEF {
        slot_set(g, object, SOURCE, native);
    }
    Ok(object)
});

std_fn!(fn reader_position(g, args) {
    let (state, _, _) = reader(g, args.this().0)?;
    Ok(enc_f64(state.position as f64))
});

std_fn!(fn reader_set_position(g, args) {
    let (state, _, view) = reader(g, args.this().0)?;
    let position = count(g, args.arg(0).0, "BinaryReader.position")?;
    if position > view.len {
        return range_error(
            g,
            &format!("BinaryReader.position: {position} is past the end, {}", view.len),
        );
    }
    state.position = position;
    Ok(UNDEF)
});

std_fn!(fn reader_remaining(g, args) {
    let (state, _, view) = reader(g, args.this().0)?;
    Ok(enc_f64(view.len.saturating_sub(state.position) as f64))
});

std_fn!(fn reader_read_array(g, args) {
    let this = args.this().0;
    reader(g, this)?;
    let length = count(g, args.arg(1).0, "BinaryReader.readArray: the count")?;
    let array = construct(g, args.arg(0).0, &[enc_f64(length as f64)])?;
    let Some(target) = typed_view(g, array).filter(|v| v.kind != TA_DATA_VIEW && v.length == length) else {
        return type_error(g, "BinaryReader.readArray: the type must be a typed array constructor");
    };
    let size = target.element_size();
    let (source, little) = take(g, this, target.len, "readArray")?;
    let out = target.bytes_mut();
    out.copy_from_slice(source);
    if size > 1 && little != NATIVE_LITTLE {
        out.chunks_exact_mut(size).for_each(<[u8]>::reverse);
    }
    Ok(array)
});

std_fn!(fn reader_read_struct(g, args) {
    let this = args.this().0;
    reader(g, this)?;
    let fields = layout(g, args.arg(0).0, "BinaryReader.readStruct")?;
    let size = fields.iter().map(|(_, s)| s.size()).sum();
    let (source, little) = take(g, this, size, "readStruct")?;
    let mut offset = 0;
    let mut values = Vec::with_capacity(fields.len());
    for (_, scalar) in &fields {
        values.push(scalar.load(&source[offset..], little));
        offset += scalar.size();
    }
    let object = new_object(g, UNDEF);
    for ((key, scalar), bits) in fields.iter().zip(values) {
        define(g, object, key, scalar.to_js(g, bits)?, 0);
    }
    Ok(object)
});

std_fn!(fn reader_bytes(g, args) {
    let this = args.this().0;
    let n = count(g, args.arg(0).0, "BinaryReader.bytes: the length")?;
    let (state, data_view, view) = reader(g, this)?;
    let offset = view.byte_offset + state.position;
    take(g, this, n, "bytes")?;
    let global = glue::rjs_global(g);
    construct(
        g,
        get(g, global, "Uint8Array"),
        &[get(g, data_view, "buffer"), enc_f64(offset as f64), enc_f64(n as f64)],
    )
});

std_fn!(fn reader_string(g, args) {
    let this = args.this().0;
    let n = count(g, args.arg(0).0, "BinaryReader.string: the length")?;
    let label = match args.arg(1).0 {
        UNDEF => "utf-8".to_owned(),
        v => string(g, v, "BinaryReader.string: the encoding")?,
    };
    let Some(encoding) = Encoding::for_label(label.as_bytes()) else {
        return range_error(g, &format!("BinaryReader.string: \"{label}\" is not an encoding this host knows"));
    };
    let (source, _) = take(g, this, n, "string")?;
    let (text, _) = encoding.decode_without_bom_handling(source);
    Ok(js_string(g, &text))
});

std_fn!(fn reader_var_uints(g, args) {
    let this = args.this().0;
    let wanted = count(g, args.arg(0).0, "BinaryReader.varUints: the count")?;
    let (state, _, view) = reader(g, this)?;
    let bytes = view.bytes();
    let mut position = state.position.min(bytes.len());
    if wanted > bytes.len() - position {
        return range_error(g, &format!("BinaryReader.varUints: {wanted} values wanted, {} bytes remain", bytes.len() - position));
    }
    let mut values = Vec::with_capacity(wanted);
    for _ in 0..wanted {
        let (value, used) = match leb128(&bytes[position..]) {
            Ok(decoded) => decoded,
            Err(reason) => return range_error(g, &format!("BinaryReader.varUints: {reason} at byte {position}")),
        };
        if value > u32::MAX as u64 {
            return range_error(g, &format!("BinaryReader.varUints: the value at byte {position} does not fit 32 bits"));
        }
        values.push(value as u32);
        position += used;
    }
    state.position = position;
    let (array, out) = typed_array(g, TA_U32, wanted)?;
    for (slot, value) in out.chunks_exact_mut(4).zip(values) {
        slot.copy_from_slice(&value.to_ne_bytes());
    }
    Ok(array)
});

struct Writer {
    len: usize,
    little: bool,
}

static WRITER: OnceLock<usize> = OnceLock::new();

unsafe fn writer<'a>(g: *mut c_void, this: u64) -> Js<&'a mut Writer> {
    match host_data::<Writer>(g, host_class::<Writer>(&WRITER, "BinaryWriter"), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a BinaryWriter"),
    }
}

unsafe fn array_buffer(g: *mut c_void, capacity: usize) -> Js<u64> {
    construct(
        g,
        get(g, glue::rjs_global(g), "ArrayBuffer"),
        &[enc_f64(capacity as f64)],
    )
}

unsafe fn append(g: *mut c_void, this: u64, bytes: &[u8]) -> Js<()> {
    let state = writer(g, this)?;
    if bytes.is_empty() {
        return Ok(());
    }
    let mut buffer = slot_get(g, this, "skathi.buffer");
    let capacity = buffer_source(g, buffer).map_or(0, |b| b.len);
    let needed = state.len + bytes.len();
    if needed > capacity {
        let grown = needed.max(capacity * 2).max(64);
        buffer = call(
            g,
            get(g, buffer, "transfer"),
            buffer,
            &[enc_f64(grown as f64)],
        )
        .map_err(|_| Thrown)?;
        slot_set(g, this, "skathi.buffer", buffer);
    }
    slot_set(g, this, "skathi.view", UNDEF);
    let Some(target) = buffer_source(g, buffer) else {
        return Err(Thrown);
    };
    target.bytes_mut()[state.len..needed].copy_from_slice(bytes);
    state.len = needed;
    Ok(())
}

std_fn!(fn writer_new(g, args) {
    let options = args.arg(0).0;
    let little = choice(g, option(g, options, "endian"), &["little", "big"], 0, "BinaryWriter: endian")? == 0;
    let capacity = match option(g, options, "initialCapacity") {
        UNDEF => 256,
        v => count(g, v, "BinaryWriter: initialCapacity")?,
    };
    let buffer = array_buffer(g, capacity)?;
    let class = host_class::<Writer>(&WRITER, "BinaryWriter");
    let object = host_create(g, class, prototype(g, "BinaryWriter"), Writer { len: 0, little });
    slot_set(g, object, "skathi.buffer", buffer);
    Ok(object)
});

std_fn!(fn writer_view(g, args) {
    let this = args.this().0;
    let state = writer(g, this)?;
    let cached = slot_get(g, this, "skathi.view");
    if is_cell(cached) {
        return Ok(cached);
    }
    let buffer = slot_get(g, this, "skathi.buffer");
    let view = construct(
        g,
        get(g, glue::rjs_global(g), "DataView"),
        &[buffer, enc_i32(0), enc_f64(state.len as f64)],
    )?;
    slot_set(g, this, "skathi.view", view);
    Ok(view)
});

std_fn!(fn writer_length(g, args) {
    Ok(enc_f64(writer(g, args.this().0)?.len as f64))
});

std_fn!(fn writer_write_all(g, args) {
    let this = args.this().0;
    let little = writer(g, this)?.little;
    let kind = choice(g, args.arg(1).0, NUMBER_TYPES, usize::MAX, "BinaryWriter.writeAll: the type")?;
    let Some(scalar) = NUMBER_SCALARS.get(kind).copied() else {
        return type_error(g, "BinaryWriter.writeAll: the type is missing");
    };
    let values = numbers(g, args.arg(0).0, "BinaryWriter.writeAll: the values")?;
    let mut bytes = Vec::with_capacity(values.len() * scalar.size());
    for value in values {
        scalar.store(scalar.bits_of(value), little, &mut bytes);
    }
    append(g, this, &bytes)?;
    Ok(this)
});

std_fn!(fn writer_write_struct(g, args) {
    let this = args.this().0;
    let little = writer(g, this)?.little;
    let fields = layout(g, args.arg(0).0, "BinaryWriter.writeStruct")?;
    let value = args.arg(1).0;
    if !is_cell(value) {
        return type_error(g, "BinaryWriter.writeStruct: the value must be an object");
    }
    let mut bytes = Vec::new();
    for (key, scalar) in fields {
        let field = get(g, value, &key);
        let bits = if scalar.wide() {
            int64_bits(g, field, &format!("BinaryWriter.writeStruct: field \"{key}\""))?
        } else {
            scalar.bits_of(number(g, field)?)
        };
        scalar.store(bits, little, &mut bytes);
    }
    append(g, this, &bytes)?;
    Ok(this)
});

std_fn!(fn writer_bytes(g, args) {
    let this = args.this().0;
    writer(g, this)?;
    let bytes = bytes_arg(g, args.arg(0).0, "BinaryWriter.bytes: the source")?.to_vec();
    append(g, this, &bytes)?;
    Ok(this)
});

std_fn!(fn writer_finish(g, args) {
    let this = args.this().0;
    let state = writer(g, this)?;
    let buffer = slot_get(g, this, "skathi.buffer");
    let exact = call(g, get(g, buffer, "transfer"), buffer, &[enc_f64(state.len as f64)])
        .map_err(|_| Thrown)?;
    let empty = array_buffer(g, 0)?;
    slot_set(g, this, "skathi.buffer", empty);
    slot_set(g, this, "skathi.view", UNDEF);
    state.len = 0;
    construct(g, get(g, glue::rjs_global(g), "Uint8Array"), &[exact])
});

struct BitReader {
    bit: usize,
    msb: bool,
}

static BIT_READER: OnceLock<usize> = OnceLock::new();

unsafe fn bit_reader<'a>(g: *mut c_void, this: u64) -> Js<(&'a mut BitReader, &'a [u8])> {
    let class = host_class::<BitReader>(&BIT_READER, "BitReader");
    let Some(state) = host_data::<BitReader>(g, class, this) else {
        return type_error(g, "this is not a BitReader");
    };
    match typed_view(g, slot_get(g, this, "skathi.bytes")) {
        Some(view) => Ok((state, view.bytes())),
        None => Err(Thrown),
    }
}

fn read_bits(state: &mut BitReader, bytes: &[u8], bits: usize) -> u32 {
    let mut value = 0u32;
    for i in 0..bits {
        let byte = bytes[state.bit >> 3];
        let at = state.bit & 7;
        if state.msb {
            value = (value << 1) | ((byte >> (7 - at)) & 1) as u32;
        } else {
            value |= (((byte >> at) & 1) as u32) << i;
        }
        state.bit += 1;
    }
    value
}

unsafe fn width(g: *mut c_void, v: u64, what: &str) -> Js<usize> {
    let bits = count(g, v, what)?;
    if !(1..=32).contains(&bits) {
        return range_error(g, &format!("{what} must be from 1 to 32"));
    }
    Ok(bits)
}

unsafe fn order(g: *mut c_void, v: u64, what: &str) -> Js<bool> {
    Ok(choice(g, v, &["msb", "lsb"], 0, what)? == 0)
}

std_fn!(fn bit_reader_new(g, args) {
    let source = args.arg(0).0;
    if typed_view(g, source).is_none() {
        return type_error(g, "BitReader: the source must be a Uint8Array");
    }
    let msb = order(g, args.arg(1).0, "BitReader: the bit order")?;
    let class = host_class::<BitReader>(&BIT_READER, "BitReader");
    let object = host_create(g, class, prototype(g, "BitReader"), BitReader { bit: 0, msb });
    slot_set(g, object, "skathi.bytes", source);
    Ok(object)
});

std_fn!(fn bit_reader_read(g, args) {
    let bits = width(g, args.arg(0).0, "BitReader.read: the number of bits")?;
    let (state, bytes) = bit_reader(g, args.this().0)?;
    let remaining = (bytes.len() * 8).saturating_sub(state.bit);
    if bits > remaining {
        return range_error(g, &format!("BitReader.read: {bits} bits wanted, {remaining} remain"));
    }
    Ok(uint(read_bits(state, bytes, bits)))
});

std_fn!(fn bit_reader_read_many(g, args) {
    let bits = width(g, args.arg(0).0, "BitReader.readMany: the number of bits")?;
    let wanted = count(g, args.arg(1).0, "BitReader.readMany: the count")?;
    let this = args.this().0;
    let (state, bytes) = bit_reader(g, this)?;
    let remaining = (bytes.len() * 8).saturating_sub(state.bit);
    if wanted > remaining / bits {
        return range_error(g, &format!("BitReader.readMany: {wanted} values of {bits} bits wanted, {remaining} bits remain"));
    }
    let values: Vec<u32> = (0..wanted).map(|_| read_bits(state, bytes, bits)).collect();
    let (array, out) = typed_array(g, TA_U32, wanted)?;
    for (slot, value) in out.chunks_exact_mut(4).zip(values) {
        slot.copy_from_slice(&value.to_ne_bytes());
    }
    Ok(array)
});

std_fn!(fn bit_reader_align(g, args) {
    let (state, _) = bit_reader(g, args.this().0)?;
    state.bit = state.bit.div_ceil(8) * 8;
    Ok(UNDEF)
});

struct BitWriter {
    bytes: Vec<u8>,
    bit: usize,
    msb: bool,
}

static BIT_WRITER: OnceLock<usize> = OnceLock::new();

unsafe fn bit_writer<'a>(g: *mut c_void, this: u64) -> Js<&'a mut BitWriter> {
    let class = host_class::<BitWriter>(&BIT_WRITER, "BitWriter");
    match host_data::<BitWriter>(g, class, this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a BitWriter"),
    }
}

fn write_bits(state: &mut BitWriter, value: u32, bits: usize) {
    for i in 0..bits {
        let at = state.bit & 7;
        if at == 0 {
            state.bytes.push(0);
        }
        let last = state.bytes.len() - 1;
        if state.msb {
            state.bytes[last] |= (((value >> (bits - 1 - i)) & 1) as u8) << (7 - at);
        } else {
            state.bytes[last] |= (((value >> i) & 1) as u8) << at;
        }
        state.bit += 1;
    }
}

std_fn!(fn bit_writer_new(g, args) {
    let msb = order(g, args.arg(0).0, "BitWriter: the bit order")?;
    let class = host_class::<BitWriter>(&BIT_WRITER, "BitWriter");
    let state = BitWriter { bytes: Vec::new(), bit: 0, msb };
    Ok(host_create(g, class, prototype(g, "BitWriter"), state))
});

std_fn!(fn bit_writer_write(g, args) {
    let this = args.this().0;
    let value = number(g, args.arg(0).0)?;
    let bits = width(g, args.arg(1).0, "BitWriter.write: the number of bits")?;
    write_bits(bit_writer(g, this)?, wrap(value) as u32, bits);
    Ok(this)
});

std_fn!(fn bit_writer_write_many(g, args) {
    let this = args.this().0;
    bit_writer(g, this)?;
    let values = numbers(g, args.arg(0).0, "BitWriter.writeMany: the values")?;
    let bits = width(g, args.arg(1).0, "BitWriter.writeMany: the number of bits")?;
    let state = bit_writer(g, this)?;
    for value in values {
        write_bits(state, wrap(value) as u32, bits);
    }
    Ok(this)
});

std_fn!(fn bit_writer_finish(g, args) {
    let state = bit_writer(g, args.this().0)?;
    let bytes = std::mem::take(&mut state.bytes);
    state.bit = 0;
    Ok(make_u8(g, &bytes))
});

unsafe fn swap(g: *mut c_void, v: u64, size: usize) -> Js<u64> {
    match typed_view(g, v).filter(|view| view.kind != TA_DATA_VIEW && view.element_size() == size) {
        Some(view) => {
            view.bytes_mut()
                .chunks_exact_mut(size)
                .for_each(<[u8]>::reverse);
            Ok(UNDEF)
        }
        None => type_error(
            g,
            &format!(
                "Endian.swap{}: the array must be a typed array of {size}-byte elements",
                size * 8
            ),
        ),
    }
}

std_fn!(fn endian_swap16(g, args) { swap(g, args.arg(0).0, 2) });
std_fn!(fn endian_swap32(g, args) { swap(g, args.arg(0).0, 4) });
std_fn!(fn endian_swap64(g, args) { swap(g, args.arg(0).0, 8) });

fn leb128(bytes: &[u8]) -> Result<(u64, usize), &'static str> {
    let mut value = 0u64;
    for (i, byte) in bytes.iter().enumerate() {
        if i == 9 && *byte > 1 {
            return Err("a value longer than 64 bits");
        }
        value |= ((byte & 0x7f) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    Err("a value cut short")
}

fn leb128_size(mut value: u64) -> usize {
    let mut size = 1;
    while value >= 0x80 {
        value >>= 7;
        size += 1;
    }
    size
}

unsafe fn var_int(g: *mut c_void, n: f64, zigzag: bool, what: &str) -> Js<u64> {
    if n.fract() != 0.0 || n.abs() > SAFE || (!zigzag && n < 0.0) {
        let wanted = if zigzag {
            "an integer"
        } else {
            "a non-negative integer; pass zigzag for signed ones"
        };
        return range_error(g, &format!("{what}: {n} is not {wanted} within 2^53"));
    }
    let value = n as i64;
    Ok(if zigzag {
        ((value << 1) ^ (value >> 63)) as u64
    } else {
        value as u64
    })
}

std_fn!(fn var_int_encode_all(g, args) {
    let values = numbers(g, args.arg(0).0, "VarInt.encodeAll: the values")?;
    let zigzag = to_bool(g, args.arg(1).0);
    let mut bytes = Vec::with_capacity(values.len());
    for n in values {
        let mut value = var_int(g, n, zigzag, "VarInt.encodeAll")?;
        while value >= 0x80 {
            bytes.push(value as u8 | 0x80);
            value >>= 7;
        }
        bytes.push(value as u8);
    }
    Ok(make_u8(g, &bytes))
});

std_fn!(fn var_int_decode_all(g, args) {
    let bytes = bytes_arg(g, args.arg(0).0, "VarInt.decodeAll: the bytes")?;
    let zigzag = to_bool(g, args.arg(1).0);
    let mut values = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let (value, used) = match leb128(&bytes[position..]) {
            Ok(decoded) => decoded,
            Err(reason) => return range_error(g, &format!("VarInt.decodeAll: {reason} at byte {position}")),
        };
        let n = if zigzag {
            ((value >> 1) as i64 ^ -((value & 1) as i64)) as f64
        } else {
            value as f64
        };
        if n.abs() > SAFE {
            return range_error(g, &format!("VarInt.decodeAll: the value at byte {position} is beyond 2^53"));
        }
        values.push(n);
        position += used;
    }
    let (array, out) = typed_array(g, TA_F64, values.len())?;
    for (slot, value) in out.chunks_exact_mut(8).zip(values) {
        slot.copy_from_slice(&value.to_ne_bytes());
    }
    Ok(array)
});

std_fn!(fn var_int_size(g, args) {
    let n = number(g, args.arg(0).0)?;
    let zigzag = to_bool(g, args.arg(1).0);
    Ok(enc_i32(leb128_size(var_int(g, n, zigzag, "VarInt.size")?) as i32))
});

const CRC32_POLY: u64 = 0xEDB8_8320;
const CRC64_XZ_POLY: u64 = 0xC96C_5795_D787_0F42;
const CRC64_NVME_POLY: u64 = 0x9A6C_9329_AC4B_C9B5;

type Tables = Box<[[u64; 256]; 8]>;

fn tables(poly: u64) -> Tables {
    let mut t: Tables = Box::new([[0; 256]; 8]);
    for i in 0..256 {
        let mut crc = i as u64;
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ poly
            } else {
                crc >> 1
            };
        }
        t[0][i] = crc;
    }
    for i in 0..256 {
        for k in 1..8 {
            let previous = t[k - 1][i];
            t[k][i] = (previous >> 8) ^ t[0][(previous & 0xff) as usize];
        }
    }
    t
}

fn crc_tables(which: usize) -> &'static Tables {
    static TABLES: [OnceLock<Tables>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    TABLES[which].get_or_init(|| tables([CRC32_POLY, CRC64_XZ_POLY, CRC64_NVME_POLY][which]))
}

/// A reflected CRC over `bytes`, slicing by eight. `crc` is the running register, not the digest.
fn crc_update(t: &Tables, mut crc: u64, bytes: &[u8]) -> u64 {
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let word = crc ^ u64::from_le_bytes(chunk.try_into().unwrap());
        crc = t[7][(word & 0xff) as usize]
            ^ t[6][((word >> 8) & 0xff) as usize]
            ^ t[5][((word >> 16) & 0xff) as usize]
            ^ t[4][((word >> 24) & 0xff) as usize]
            ^ t[3][((word >> 32) & 0xff) as usize]
            ^ t[2][((word >> 40) & 0xff) as usize]
            ^ t[1][((word >> 48) & 0xff) as usize]
            ^ t[0][(word >> 56) as usize];
    }
    for byte in chunks.remainder() {
        crc = t[0][((crc ^ *byte as u64) & 0xff) as usize] ^ (crc >> 8);
    }
    crc
}

fn crc32(seed: u32, bytes: &[u8]) -> u32 {
    !(crc_update(crc_tables(0), (!seed) as u64, bytes) as u32)
}

const CRC64_VARIANTS: &[&str] = &["ecma", "nvme"];

struct Crc32 {
    register: u64,
}

struct Crc64 {
    register: u64,
    variant: usize,
}

static CRC32: OnceLock<usize> = OnceLock::new();
static CRC64: OnceLock<usize> = OnceLock::new();

std_fn!(fn crc32_compute(g, args) {
    let bytes = bytes_arg(g, args.arg(0).0, "CRC32.compute: the bytes")?;
    let seed = match args.arg(1).0 {
        UNDEF => 0,
        v => wrap(number(g, v)?) as u32,
    };
    Ok(uint(crc32(seed, bytes)))
});

std_fn!(fn crc32_create(g, _args) {
    let class = host_class::<Crc32>(&CRC32, "CRC32Stream");
    let state = Crc32 { register: u32::MAX as u64 };
    Ok(host_create(g, class, prototype(g, "CRC32Stream"), state))
});

unsafe fn crc32_state<'a>(g: *mut c_void, this: u64) -> Js<&'a mut Crc32> {
    match host_data::<Crc32>(g, host_class::<Crc32>(&CRC32, "CRC32Stream"), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a CRC32 stream"),
    }
}

std_fn!(fn crc32_stream_update(g, args) {
    let state = crc32_state(g, args.this().0)?;
    let bytes = bytes_arg(g, args.arg(0).0, "CRC32 update: the bytes")?;
    state.register = crc_update(crc_tables(0), state.register, bytes);
    Ok(UNDEF)
});

std_fn!(fn crc32_stream_digest(g, args) {
    Ok(uint(!(crc32_state(g, args.this().0)?.register as u32)))
});

std_fn!(fn crc64_compute(g, args) {
    let bytes = bytes_arg(g, args.arg(0).0, "CRC64.compute: the bytes")?;
    let variant = choice(g, args.arg(1).0, CRC64_VARIANTS, 0, "CRC64.compute: the variant")?;
    big_uint(g, !crc_update(crc_tables(1 + variant), u64::MAX, bytes))
});

std_fn!(fn crc64_create(g, args) {
    let variant = choice(g, args.arg(0).0, CRC64_VARIANTS, 0, "CRC64.create: the variant")?;
    let class = host_class::<Crc64>(&CRC64, "CRC64Stream");
    let state = Crc64 { register: u64::MAX, variant };
    Ok(host_create(g, class, prototype(g, "CRC64Stream"), state))
});

unsafe fn crc64_state<'a>(g: *mut c_void, this: u64) -> Js<&'a mut Crc64> {
    match host_data::<Crc64>(g, host_class::<Crc64>(&CRC64, "CRC64Stream"), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a CRC64 stream"),
    }
}

std_fn!(fn crc64_stream_update(g, args) {
    let state = crc64_state(g, args.this().0)?;
    let bytes = bytes_arg(g, args.arg(0).0, "CRC64 update: the bytes")?;
    state.register = crc_update(crc_tables(1 + state.variant), state.register, bytes);
    Ok(UNDEF)
});

std_fn!(fn crc64_stream_digest(g, args) {
    big_uint(g, !crc64_state(g, args.this().0)?.register)
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let reader = support::class(
        g,
        &ClassSpec {
            name: "BinaryReader",
            arity: 1,
            construct: Some(reader_new),
            parent_proto: 0,
            methods: &[
                ("readArray", 2, reader_read_array),
                ("readStruct", 1, reader_read_struct),
                ("bytes", 1, reader_bytes),
                ("string", 1, reader_string),
                ("varUints", 1, reader_var_uints),
            ],
            getters: &[("remaining", reader_remaining)],
            accessors: &[("position", reader_position, reader_set_position)],
        },
    );
    let writer = support::class(
        g,
        &ClassSpec {
            name: "BinaryWriter",
            arity: 0,
            construct: Some(writer_new),
            parent_proto: 0,
            methods: &[
                ("writeAll", 2, writer_write_all),
                ("writeStruct", 2, writer_write_struct),
                ("bytes", 1, writer_bytes),
                ("finish", 0, writer_finish),
            ],
            getters: &[("view", writer_view), ("length", writer_length)],
            accessors: &[],
        },
    );
    let bit_reader = support::class(
        g,
        &ClassSpec {
            name: "BitReader",
            arity: 1,
            construct: Some(bit_reader_new),
            parent_proto: 0,
            methods: &[
                ("read", 1, bit_reader_read),
                ("readMany", 2, bit_reader_read_many),
                ("align", 0, bit_reader_align),
            ],
            getters: &[],
            accessors: &[],
        },
    );
    let bit_writer = support::class(
        g,
        &ClassSpec {
            name: "BitWriter",
            arity: 0,
            construct: Some(bit_writer_new),
            parent_proto: 0,
            methods: &[
                ("write", 2, bit_writer_write),
                ("writeMany", 2, bit_writer_write_many),
                ("finish", 0, bit_writer_finish),
            ],
            getters: &[],
            accessors: &[],
        },
    );
    support::class(
        g,
        &ClassSpec {
            name: "CRC32Stream",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("update", 1, crc32_stream_update),
                ("digest", 0, crc32_stream_digest),
            ],
            getters: &[],
            accessors: &[],
        },
    );
    support::class(
        g,
        &ClassSpec {
            name: "CRC64Stream",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("update", 1, crc64_stream_update),
                ("digest", 0, crc64_stream_digest),
            ],
            getters: &[],
            accessors: &[],
        },
    );

    let endian = namespace(
        g,
        "Endian",
        &[
            ("swap16", 1, endian_swap16),
            ("swap32", 1, endian_swap32),
            ("swap64", 1, endian_swap64),
        ],
    );
    let native = if NATIVE_LITTLE { "little" } else { "big" };
    define(g, endian, "native", js_string(g, native), 1 | 4);
    let var_int = namespace(
        g,
        "VarInt",
        &[
            ("encodeAll", 1, var_int_encode_all),
            ("decodeAll", 1, var_int_decode_all),
            ("size", 1, var_int_size),
        ],
    );
    let crc32 = namespace(
        g,
        "CRC32",
        &[("compute", 1, crc32_compute), ("create", 0, crc32_create)],
    );
    let crc64 = namespace(
        g,
        "CRC64",
        &[("compute", 1, crc64_compute), ("create", 0, crc64_create)],
    );

    let module = new_object(g, UNDEF);
    define(g, module, "BinaryReader", reader.ctor, 0);
    define(g, module, "BinaryWriter", writer.ctor, 0);
    define(g, module, "BitReader", bit_reader.ctor, 0);
    define(g, module, "BitWriter", bit_writer.ctor, 0);
    define(g, module, "Endian", endian, 0);
    define(g, module, "VarInt", var_int, 0);
    define(g, module, "CRC32", crc32, 0);
    define(g, module, "CRC64", crc64, 0);
    define(g, std, "binary", module, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crcs_match_their_published_check_values() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
        let xz = !crc_update(crc_tables(1), u64::MAX, b"123456789");
        assert_eq!(xz, 0x995D_C9BB_DF19_39FA);
        let nvme = !crc_update(crc_tables(2), u64::MAX, b"123456789");
        assert_eq!(nvme, 0xAE8B_1486_0A79_9888);
    }

    #[test]
    fn slicing_by_eight_agrees_with_the_bytewise_form() {
        let bytes: Vec<u8> = (0..1027u32).map(|i| (i * 31 + 7) as u8).collect();
        let t = crc_tables(1);
        let mut bytewise = u64::MAX;
        for byte in &bytes {
            bytewise = t[0][((bytewise ^ *byte as u64) & 0xff) as usize] ^ (bytewise >> 8);
        }
        assert_eq!(crc_update(t, u64::MAX, &bytes), bytewise);
    }

    #[test]
    fn bits_round_trip_in_both_orders() {
        for msb in [true, false] {
            let mut writer = BitWriter {
                bytes: Vec::new(),
                bit: 0,
                msb,
            };
            write_bits(&mut writer, 0b101, 3);
            write_bits(&mut writer, 0x1ABCD, 17);
            write_bits(&mut writer, u32::MAX, 32);
            let mut reader = BitReader { bit: 0, msb };
            assert_eq!(read_bits(&mut reader, &writer.bytes, 3), 0b101);
            assert_eq!(read_bits(&mut reader, &writer.bytes, 17), 0x1ABCD);
            assert_eq!(read_bits(&mut reader, &writer.bytes, 32), u32::MAX);
        }
        let mut writer = BitWriter {
            bytes: Vec::new(),
            bit: 0,
            msb: true,
        };
        write_bits(&mut writer, 0b101, 3);
        assert_eq!(writer.bytes, [0b1010_0000]);
    }

    #[test]
    fn leb128_rejects_what_does_not_fit() {
        assert_eq!(leb128(&[0xE5, 0x8E, 0x26]), Ok((624_485, 3)));
        assert!(leb128(&[0x80]).is_err());
        assert!(leb128(&[0xff; 10]).is_err());
        let mut max = vec![0xff; 9];
        max.push(1);
        assert_eq!(leb128(&max), Ok((u64::MAX, 10)));
    }
}
