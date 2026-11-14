use super::*;

/// `testdata/compound.ivf`: 48 frames of ffmpeg's `testsrc2` at 72×56, turning, VP9 two-pass with
/// alt-ref frames, 5 KB. 8 hidden alt-ref frames ride in superframes, and the turn makes sub-pixel
/// motion and compound prediction that reach the SIMD shims. Made with ffmpeg 6.0 (libvpx-vp9):
///
/// ```text
/// ffmpeg -f lavfi -i testsrc2=size=72x56:rate=30:duration=1.6 -vf rotate=0.05*t*30:c=black \
///   -c:v libvpx-vp9 -b:v 20k -auto-alt-ref 1 -lag-in-frames 25 -pass 1 -f null NUL
/// (the same with -pass 2 compound.ivf)
/// ```
///
/// An unrotated `testsrc2` moves by whole pixels (no shim is reached), and a clip much shorter than
/// 1.6 s gets no alt-ref frame.
const COMPOUND: &[u8] = include_bytes!("../testdata/compound.ivf");

/// FNV-1a 64 of every shown frame's visible Y, U and V rows, in order: what ffmpeg's native VP9
/// decoder and its libvpx one give, and the pure-C build here (`SK_VPX_CONFIG=generic`).
const COMPOUND_HASH: u64 = 0x22e8_10e0_5c5b_9e2b;
const COMPOUND_FRAMES: usize = 48;

/// The packets of an IVF file: a 32-byte file header, then per frame a 12-byte header and the packet.
pub(crate) fn ivf(bytes: &[u8]) -> Vec<&[u8]> {
    assert_eq!(&bytes[..4], b"DKIF");
    let mut packets = Vec::new();
    let mut pos = 32;
    while pos + 12 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 12;
        packets.push(&bytes[pos..pos + size]);
        pos += size;
    }
    packets
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash = (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn hash_frame(mut hash: u64, frame: &Frame<'_>) -> u64 {
    for plane in &frame.planes {
        for y in 0..plane.height {
            hash = fnv(hash, plane.row(y));
        }
    }
    hash
}

fn decode_all(packets: &[&[u8]]) -> (usize, u64) {
    let mut decoder = Decoder::new().unwrap();
    let mut hash = FNV_OFFSET;
    let mut shown = 0;
    for packet in packets {
        if decoder.decode(packet).unwrap() {
            let frame = decoder.frame().unwrap();
            hash = hash_frame(hash, &frame);
            shown += 1;
        }
    }
    (shown, hash)
}

/// A VP9 superframe ends in an index whose marker byte is 0b110xxxxx, repeated
/// at its start (VP9 bitstream spec, Annex B).
fn is_superframe(packet: &[u8]) -> bool {
    let last = *packet.last().unwrap();
    if last & 0xe0 != 0xc0 {
        return false;
    }
    let frames = (last & 7) as usize + 1;
    let size_bytes = ((last >> 3) & 3) as usize + 1;
    let index = 2 + size_bytes * frames;
    packet.len() >= index && packet[packet.len() - index] == last
}

#[test]
fn the_c_layout_is_the_one_declared_here() {
    use std::mem::{offset_of, size_of};
    let c = |which| unsafe { ffi::sk_vpx_layout(which) };
    assert_eq!(c(0), size_of::<ffi::vpx_codec_ctx_t>());
    assert_eq!(c(1), offset_of!(ffi::vpx_codec_ctx_t, err));
    assert_eq!(c(2), offset_of!(ffi::vpx_codec_ctx_t, priv_));
    assert_eq!(c(3), size_of::<ffi::vpx_codec_dec_cfg_t>());
    assert_eq!(c(4), size_of::<ffi::vpx_codec_stream_info_t>());
    assert_eq!(c(5), size_of::<ffi::vpx_image_t>());
    assert_eq!(c(6), offset_of!(ffi::vpx_image_t, d_w));
    assert_eq!(c(7), offset_of!(ffi::vpx_image_t, x_chroma_shift));
    assert_eq!(c(8), offset_of!(ffi::vpx_image_t, planes));
    assert_eq!(c(9), offset_of!(ffi::vpx_image_t, stride));
    assert_eq!(c(10), offset_of!(ffi::vpx_image_t, fb_priv));
    assert_eq!(c(11), ffi::VPX_DECODER_ABI_VERSION as usize);
}

#[test]
fn it_is_the_vendored_release() {
    assert_eq!(version(), "v1.17.0");
}

#[test]
fn the_compound_clip_decodes_frame_by_frame() {
    let packets = ivf(COMPOUND);
    assert_eq!(packets.len(), COMPOUND_FRAMES);
    let mut decoder = Decoder::new().unwrap();
    for (i, packet) in packets.iter().enumerate() {
        assert!(decoder.decode(packet).unwrap(), "packet {i} shows a frame");
        let frame = decoder.frame().unwrap();
        assert_eq!((frame.width, frame.height, frame.format), (72, 56, Format::I420));
        assert_eq!((frame.planes[1].width, frame.planes[1].height), (36, 28));
        assert_eq!(frame.color_space, ColorSpace::Unknown);
        assert!(!frame.full_range);
    }
    let superframes = packets.iter().filter(|p| is_superframe(p)).count();
    assert!(superframes > 0, "the clip has no alt-ref superframe");
}

#[test]
fn the_compound_clip_is_bit_exact() {
    let (shown, hash) = decode_all(&ivf(COMPOUND));
    assert_eq!(shown, COMPOUND_FRAMES);
    println!("compound.ivf hash {hash:#018x} (config {})", if cfg!(sk_vpx_x86) { "x86_64-msvc" } else { "generic" });
    if COMPOUND_HASH != 0 {
        assert_eq!(hash, COMPOUND_HASH, "decoded frames differ from the pure-C build");
    }
}

#[test]
fn peek_reads_a_keyframe_size() {
    let packets = ivf(COMPOUND);
    assert_eq!(peek(packets[0]), Some(StreamInfo { width: 72, height: 56, keyframe: true }));
    assert_eq!(peek(b"\x00\x01\x02"), None);
}

#[test]
fn memory_is_charged_to_the_decoder_that_allocated_it() {
    let packets = ivf(COMPOUND);
    let mut a = Decoder::new().unwrap();
    let b = Decoder::new().unwrap();
    let before = a.memory();
    assert!(before.live > 0, "a new decoder holds its context");
    let b_before = b.memory();
    for packet in &packets {
        a.decode(packet).unwrap();
    }
    let after = a.memory();
    assert!(after.live > before.live, "frame buffers are charged: {before:?} -> {after:?}");
    assert!(after.peak >= after.live);
    assert_eq!(b.memory(), b_before, "another decoder's allocations are not b's");
}

#[test]
fn a_memory_cap_fails_the_decode_and_not_the_process() {
    let packets = ivf(COMPOUND);
    let mut decoder = Decoder::new().unwrap();
    let held = decoder.memory().live;
    decoder.set_memory_limit(held + 4096);
    let err = decoder.decode(packets[0]).unwrap_err();
    assert_eq!(err.code, ErrorCode::Mem, "{err}");
    assert!(decoder.memory().live <= held + 4096);
    decoder.set_memory_limit(0);
    assert!(decoder.decode(packets[0]).unwrap());
}

pub(crate) struct Noise(u64);

impl Noise {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

#[test]
fn garbage_truncation_and_bit_flips_are_errors_not_crashes() {
    let packets = ivf(COMPOUND);
    let mut noise = Noise(0x9e37_79b9_7f4a_7c15);
    let mut decoder = Decoder::new().unwrap();
    assert!(decoder.decode(&[]).is_err());
    for n in [1, 2, 3, 10, 100, 1000] {
        let _ = decoder.decode(&noise.bytes(n));
    }
    for cut in 1..packets[0].len() {
        let _ = decoder.decode(&packets[0][..cut]);
    }
    for round in 0..40 {
        let mut decoder = Decoder::new().unwrap();
        for packet in &packets {
            let mut damaged = packet.to_vec();
            for _ in 0..1 + round % 4 {
                let at = noise.next() as usize % damaged.len();
                damaged[at] ^= 1 << (noise.next() % 8);
            }
            if decoder.decode(&damaged).unwrap_or(false) {
                let frame = decoder.frame().unwrap();
                let _ = hash_frame(FNV_OFFSET, &frame);
            }
        }
    }
    for packet in &packets {
        assert!(decoder.decode(packet).unwrap());
    }
}

#[test]
fn decoders_on_many_threads_at_once() {
    let expected = decode_all(&ivf(COMPOUND));
    let threads: Vec<_> = (0..8).map(|_| std::thread::spawn(|| decode_all(&ivf(COMPOUND)))).collect();
    for t in threads {
        assert_eq!(t.join().unwrap(), expected);
    }
}

#[test]
fn a_decoder_moves_between_threads() {
    fn send<T: Send>() {}
    send::<Decoder>();
    let packets: Vec<Vec<u8>> = ivf(COMPOUND).into_iter().map(<[u8]>::to_vec).collect();
    let mut decoder = Decoder::new().unwrap();
    decoder.decode(&packets[0]).unwrap();
    let decoder = std::thread::spawn(move || {
        let mut decoder = decoder;
        decoder.decode(&packets[1]).unwrap();
        decoder
    })
    .join()
    .unwrap();
    assert!(decoder.frame().is_some());
}

#[cfg(sk_vpx_x86)]
mod x86;
