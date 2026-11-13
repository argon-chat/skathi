//! The decoder behind the contract: the checked-in clips decoded to the
//! hashes other decoders give, the frame walk, conversion and its alpha,
//! damage, threads; and (ignored) the real samples and the benchmark.

use super::webm::tests::{cluster, file, info, master, simple_block, size, video_track};
use super::*;

const ALPHA: &[u8] = include_bytes!("testdata/alpha.webm");
const SUPERFRAME: &[u8] = include_bytes!("testdata/superframe.webm");

/// vn-vpx-sys's `testdata/compound.ivf` decoded (its `COMPOUND_HASH`, also the pure-C build's and
/// ffmpeg's): `superframe.webm` is that clip remuxed by ffmpeg.
const COMPOUND_HASH: u64 = 0x22e8_10e0_5c5b_9e2b;
/// ffmpeg 6.0 with libvpx-vp9 on `alpha.webm` (`-c:v libvpx-vp9 -i alpha.webm -f rawvideo -pix_fmt
/// yuva420p`): FNV-1a over every frame's Y, U and V rows, and over every frame's A rows.
const ALPHA_COLOUR_HASH: u64 = 0x92e8_01d8_e6cf_69ed;
const ALPHA_ALPHA_HASH: u64 = 0xef48_4eae_1fef_c6d0;

const TRACKS: u32 = 0x1654_AE6B;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash = (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn hash_planes(mut hash: u64, frame: &vn_vpx_sys::Frame<'_>, planes: usize) -> u64 {
    for plane in &frame.planes[..planes] {
        for y in 0..plane.height {
            hash = fnv(hash, plane.row(y));
        }
    }
    hash
}

fn open(bytes: &[u8]) -> Arc<VideoSource> {
    VideoSource::open(Arc::from(bytes)).unwrap()
}

/// Decodes every frame: (frames, hash of the colour planes, hash of the alpha planes).
fn decode_hashes(source: &Arc<VideoSource>, options: DecodeOptions) -> (u32, u64, u64) {
    let mut d = source.decoder(options).unwrap();
    let (mut colour, mut alpha) = (FNV_OFFSET, FNV_OFFSET);
    let mut n = 0;
    while let Some(i) = d.decode_next().unwrap() {
        assert_eq!(i, n);
        if let Some(f) = d.color.as_ref().and_then(Decoder::frame) {
            colour = hash_planes(colour, &f, 3);
        }
        if d.alpha_now {
            alpha = hash_planes(alpha, &d.alpha.as_ref().and_then(Decoder::frame).unwrap(), 1);
        }
        n += 1;
    }
    (n, colour, alpha)
}

fn convert(d: &mut VideoDecoder, width: u32, height: u32) -> Result<Vec<u8>, VideoError> {
    let mut pixels = vec![0; (width * height * 4) as usize];
    d.convert(&mut Target { pixels: &mut pixels, width, height })?;
    Ok(pixels)
}

#[test]
fn the_clips_decode_to_what_ffmpeg_and_libvpx_decode() {
    assert_eq!(decode_hashes(&open(ALPHA), DecodeOptions::default()), (48, ALPHA_COLOUR_HASH, ALPHA_ALPHA_HASH));
    assert_eq!(decode_hashes(&open(SUPERFRAME), DecodeOptions::default()), (48, COMPOUND_HASH, FNV_OFFSET));
}

#[test]
fn a_tinted_decoder_decodes_only_the_alpha() {
    let source = open(ALPHA);
    assert_eq!(decode_hashes(&source, DecodeOptions { alpha_only: true }), (48, FNV_OFFSET, ALPHA_ALPHA_HASH));
    let mut full = source.decoder(DecodeOptions::default()).unwrap();
    let mut tinted = source.decoder(DecodeOptions { alpha_only: true }).unwrap();
    assert!(tinted.color.is_none());
    for _ in 0..10 {
        full.decode_next().unwrap();
        tinted.decode_next().unwrap();
    }
    assert!(tinted.memory_bytes() < full.memory_bytes());
    // Its alpha is the colour decoder's, at every size; its colour is white.
    for (w, h) in [(41, 29), (20, 14), (82, 58), (7, 3)] {
        let colour = convert(&mut full, w, h).unwrap();
        let white = convert(&mut tinted, w, h).unwrap();
        for (c, t) in colour.chunks_exact(4).zip(white.chunks_exact(4)) {
            assert_eq!(t, &[255, 255, 255, c[3]], "{w}x{h}");
        }
    }
}

#[test]
fn a_frame_converts_to_straight_bgra_with_its_alpha_as_is() {
    let source = open(ALPHA);
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    for _ in 0..20 {
        d.decode_next().unwrap();
    }
    let pixels = convert(&mut d, 41, 29).unwrap();
    let frame = d.color.as_ref().and_then(Decoder::frame).unwrap();
    let alpha = d.alpha.as_ref().and_then(Decoder::frame).unwrap();
    for y in 0..29 {
        for x in 0..41 {
            assert_eq!(pixels[(y * 41 + x) * 4 + 3], alpha.planes[0].row(y)[x]);
        }
    }
    assert!(pixels.chunks_exact(4).any(|p| p[3] == 0) && pixels.chunks_exact(4).any(|p| p[3] == 255), "the clip has both");
    // The colour: ffmpeg wrote a Colour with a Range alone and libvpx-vp9 an
    // unknown color_space, so Chromium reads it as BT.601 studio range.
    assert_eq!(frame.color_space, vn_vpx_sys::ColorSpace::Unknown);
    let plane = yuv_plane;
    let i420 = vn_yuv::I420 {
        width: 41,
        height: 29,
        y: plane(&frame.planes[0]),
        u: plane(&frame.planes[1]),
        v: plane(&frame.planes[2]),
        a: Some(plane(&alpha.planes[0])),
    };
    let mut want = vec![0; 41 * 29 * 4];
    let bt601 = vn_yuv::Colorimetry::new(vn_yuv::Matrix::Bt601, false);
    vn_yuv::Converter::new().to_bgra(&i420, bt601, &mut want, 41, 29).unwrap();
    assert_eq!(pixels, want);
}

#[test]
fn frames_come_in_order_then_none_and_a_rewind_starts_over() {
    let source = open(ALPHA);
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    assert!(matches!(convert(&mut d, 8, 8), Err(VideoError::Malformed(_))), "nothing decoded yet");
    assert_eq!(d.decode_next(), Ok(Some(0)));
    let first = convert(&mut d, 41, 29).unwrap();
    for i in 1..48 {
        assert_eq!(d.decode_next(), Ok(Some(i)));
        assert_eq!(d.position(), i + 1);
    }
    assert_eq!(d.decode_next(), Ok(None));
    assert_eq!(d.decode_next(), Ok(None));
    assert_eq!(d.position(), 48);
    let last = convert(&mut d, 41, 29).expect("past the end, the last frame stays");
    assert_ne!(first, last);
    d.rewind().unwrap();
    assert_eq!(d.position(), 0);
    assert!(convert(&mut d, 8, 8).is_err(), "a rewind drops the frame");
    assert_eq!(d.decode_next(), Ok(Some(0)));
    assert_eq!(convert(&mut d, 41, 29).unwrap(), first);
}

#[test]
fn a_file_without_alpha_is_opaque_and_tinted_it_is_white() {
    let source = open(SUPERFRAME);
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    assert!(d.alpha.is_none());
    d.decode_next().unwrap();
    assert!(convert(&mut d, 36, 28).unwrap().chunks_exact(4).all(|p| p[3] == 255));
    // No colour space and no alpha: BT.709 studio range, as Chrome shows it with a GPU (colour.rs).
    let frame = d.color.as_ref().and_then(Decoder::frame).unwrap();
    assert_eq!(frame.color_space, vn_vpx_sys::ColorSpace::Unknown);
    let i420 = vn_yuv::I420 {
        width: 72,
        height: 56,
        y: yuv_plane(&frame.planes[0]),
        u: yuv_plane(&frame.planes[1]),
        v: yuv_plane(&frame.planes[2]),
        a: None,
    };
    let mut want = vec![0; 72 * 56 * 4];
    let bt709 = vn_yuv::Colorimetry::new(vn_yuv::Matrix::Bt709, false);
    vn_yuv::Converter::new().to_bgra(&i420, bt709, &mut want, 72, 56).unwrap();
    assert_eq!(convert(&mut d, 72, 56).unwrap(), want);
    let mut tinted = source.decoder(DecodeOptions { alpha_only: true }).unwrap();
    assert!(tinted.color.is_none() && tinted.alpha.is_none());
    assert!(tinted.memory_bytes() < 4096);
    let mut n = 0;
    while tinted.decode_next().unwrap().is_some() {
        n += 1;
    }
    assert_eq!(n, 48);
    assert!(convert(&mut tinted, 10, 10).unwrap().iter().all(|&b| b == 255));
}

/// `superframe.webm` rebuilt with every frame in a block of its own, and `damage` applied to the packets.
fn split_frames(damage: impl Fn(usize, &[u8]) -> Vec<u8>) -> Vec<u8> {
    let webm = webm::demux(SUPERFRAME).unwrap();
    let mut blocks = Vec::new();
    for b in &webm.blocks {
        let packet = b.data.of(SUPERFRAME);
        for (s, e) in vp9::frames(packet).unwrap().iter() {
            let i = blocks.len();
            blocks.push(simple_block(1, b.time as i16, &damage(i, &packet[s..e])));
        }
    }
    file(&[info(1_000_000, Some(1600.0)), master(TRACKS, &[video_track(1, "V_VP9", &size(72, 56))]), cluster(0, &blocks)])
}

#[test]
fn a_lone_hidden_frame_is_decoded_with_the_next_shown_one() {
    let bytes = split_frames(|_, p| p.to_vec());
    let source = open(&bytes);
    assert_eq!(source.packets.len(), 56, "48 shown and 8 hidden, each a block");
    assert_eq!(source.meta().frame_count(), 48);
    assert_eq!(decode_hashes(&source, DecodeOptions::default()), (48, COMPOUND_HASH, FNV_OFFSET));
}

#[test]
fn a_damaged_frame_stops_the_decoder_until_a_rewind() {
    let bytes = split_frames(|i, p| if i == 12 { p[..p.len() / 2].to_vec() } else { p.to_vec() });
    let source = open(&bytes);
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    let mut decoded = 0;
    let error = loop {
        match d.decode_next() {
            Ok(Some(_)) => decoded += 1,
            Ok(None) => panic!("the damage went unnoticed"),
            Err(e) => break e,
        }
    };
    assert!(matches!(&error, VideoError::Malformed(why) if why.contains(&format!("video frame {decoded}"))), "{error}");
    assert!(d.decode_next().unwrap_err().to_string().contains("rewind"));
    assert!(convert(&mut d, 8, 8).is_err());
    d.rewind().unwrap();
    for i in 0..decoded {
        assert_eq!(d.decode_next(), Ok(Some(i)), "the frames before the damage decode again");
    }
}

#[test]
fn decoders_on_several_threads_decode_the_same_frames() {
    let source = open(ALPHA);
    let want = decode_hashes(&source, DecodeOptions::default());
    let threads: Vec<_> = (0..6)
        .map(|i| {
            let source = source.clone();
            std::thread::spawn(move || decode_hashes(&source, DecodeOptions { alpha_only: i % 2 == 1 }))
        })
        .collect();
    for (i, t) in threads.into_iter().enumerate() {
        let got = t.join().unwrap();
        if i % 2 == 1 {
            assert_eq!(got, (want.0, FNV_OFFSET, want.2));
        } else {
            assert_eq!(got, want);
        }
    }
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    for i in 0..4 {
        d = std::thread::spawn(move || {
            assert_eq!(d.decode_next(), Ok(Some(i)));
            d
        })
        .join()
        .unwrap();
    }
}

#[test]
fn a_decoder_counts_libvpx_and_its_scratch() {
    let source = open(ALPHA);
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    d.decode_next().unwrap();
    convert(&mut d, 20, 14).unwrap();
    let libvpx: usize = [&d.color, &d.alpha].iter().filter_map(|x| x.as_ref()).map(|x| x.memory().live).sum();
    assert!(libvpx > 100_000, "two VP9 contexts: {libvpx}");
    assert!(d.memory_bytes() > libvpx + d.converter.memory_bytes() - 1);
}

/// `VN_STICKER_SAMPLES=<dir> cargo test -p vn-sticker samples_decode -- --ignored --nocapture`:
/// decodes every `.webm` in a directory and prints what each frame is read as.
#[test]
#[ignore]
fn samples_decode() {
    let dir = std::env::var("VN_STICKER_SAMPLES").expect("VN_STICKER_SAMPLES=<dir of .webm>");
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths.iter().filter(|p| p.extension().is_some_and(|e| e == "webm")) {
        let source = match VideoSource::open(Arc::from(std::fs::read(path).unwrap())) {
            Ok(s) => s,
            Err(e) => {
                println!("{}: {e}", path.display());
                continue;
            }
        };
        let mut d = source.decoder(DecodeOptions::default()).unwrap();
        let mut frames = 0;
        let mut first = None;
        let result = loop {
            match d.decode_next() {
                Ok(Some(_)) => {
                    frames += 1;
                    if first.is_none() {
                        let f = d.color.as_ref().and_then(Decoder::frame).unwrap();
                        let rule = colour::colorimetry(source.colour.as_ref(), f.color_space, f.full_range, source.meta.has_alpha);
                        first = Some((f.color_space, f.full_range, rule));
                    }
                    let m = source.meta();
                    convert(&mut d, m.width, m.height).unwrap();
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        println!(
            "{}: {frames} frames, {:?}, {} KB decoding, {result:?}",
            path.display(),
            first,
            d.memory_bytes() / 1024
        );
    }
}

/// `VN_STICKER_DUMP="<file.webm>|<out>|<frame>,<frame>,...[|601s|601f|709s|709f]"
/// cargo test -p vn-sticker dump_frames -- --ignored --nocapture`: writes the frames, converted at
/// the file's own size (straight BGRA), into `<out>` and prints their pts, to compare with a
/// browser. The last part reads them with that matrix and range instead of the colour rule's.
#[test]
#[ignore]
fn dump_frames() {
    let spec = std::env::var("VN_STICKER_DUMP").expect("VN_STICKER_DUMP=<file.webm>|<out>|<frames>");
    let mut parts = spec.split('|');
    let (input, output, frames) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
    let frames: Vec<u32> = frames.split(',').map(|f| f.parse().unwrap()).collect();
    let forced = parts
        .next()
        .map(|c| vn_yuv::Colorimetry::new(if c.starts_with("709") { vn_yuv::Matrix::Bt709 } else { vn_yuv::Matrix::Bt601 }, c.ends_with('f')));
    let source = VideoSource::open(Arc::from(std::fs::read(input).unwrap())).unwrap();
    let m = source.meta().clone();
    let mut d = source.decoder(DecodeOptions::default()).unwrap();
    let mut out = Vec::new();
    while let Some(i) = d.decode_next().unwrap() {
        if !frames.contains(&i) {
            continue;
        }
        let frame = d.color.as_ref().and_then(Decoder::frame).unwrap();
        let rule = colour::colorimetry(source.colour.as_ref(), frame.color_space, frame.full_range, m.has_alpha);
        println!("frame {i} pts {} ms, {:?} read as {:?}", m.pts_ms[i as usize], frame.color_space, forced.unwrap_or(rule));
        let Some(colorimetry) = forced else {
            out.extend(convert(&mut d, m.width, m.height).unwrap());
            continue;
        };
        let alpha = if d.alpha_now { d.alpha.as_ref().and_then(Decoder::frame) } else { None };
        let i420 = vn_yuv::I420 {
            width: frame.width,
            height: frame.height,
            y: yuv_plane(&frame.planes[0]),
            u: yuv_plane(&frame.planes[1]),
            v: yuv_plane(&frame.planes[2]),
            a: alpha.map(|a| yuv_plane(&a.planes[0])),
        };
        let mut pixels = vec![0; (m.width * m.height * 4) as usize];
        vn_yuv::Converter::new().to_bgra(&i420, colorimetry, &mut pixels, m.width, m.height).unwrap();
        out.extend(pixels);
    }
    std::fs::write(output, out).unwrap();
}

/// `VN_STICKER_BENCH=<dir> cargo test --release -p vn-sticker bench -- --ignored --nocapture`: per
/// `.webm`, one thread, decode, convert and total per frame, and the decoder's memory.
#[test]
#[ignore]
fn bench() {
    use std::time::Instant;
    let dir = std::env::var("VN_STICKER_BENCH").expect("VN_STICKER_BENCH=<dir of .webm>");
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    println!("vn-yuv level: {}", vn_yuv::Level::best().name());
    for path in paths.iter().filter(|p| p.extension().is_some_and(|e| e == "webm")) {
        let source = VideoSource::open(Arc::from(std::fs::read(path).unwrap())).unwrap();
        let m = source.meta().clone();
        let side = m.width.max(m.height);
        let targets: Vec<u32> = if side >= 256 { vec![side, 256, 160] } else { vec![side, 48, 32] };
        println!("{}: {}x{}, {} frames, alpha {}", path.display(), m.width, m.height, m.frame_count(), m.has_alpha);
        for alpha_only in [false, true] {
            let mut d = source.decoder(DecodeOptions { alpha_only }).unwrap();
            let mut decode = f64::MAX;
            for _ in 0..5 {
                d.rewind().unwrap();
                let start = Instant::now();
                while d.decode_next().unwrap().is_some() {}
                decode = decode.min(start.elapsed().as_secs_f64() * 1e3 / m.frame_count() as f64);
            }
            let memory = d.memory_bytes();
            let mut line = format!("  {:<12} decode {decode:.3} ms", if alpha_only { "alpha only" } else { "colour+alpha" });
            for &t in &targets {
                let (tw, th) = ((m.width * t / side).max(1), (m.height * t / side).max(1));
                let mut pixels = vec![0; (tw * th * 4) as usize];
                let mut best = f64::MAX;
                for _ in 0..5 {
                    d.rewind().unwrap();
                    let mut spent = 0.0;
                    while d.decode_next().unwrap().is_some() {
                        let start = Instant::now();
                        d.convert(&mut Target { pixels: &mut pixels, width: tw, height: th }).unwrap();
                        spent += start.elapsed().as_secs_f64();
                    }
                    best = best.min(spent * 1e3 / m.frame_count() as f64);
                }
                line += &format!(" | {tw}x{th}: convert {best:.3}, total {:.3} ms", decode + best);
            }
            println!("{line} | memory {:.2} MB", memory as f64 / (1 << 20) as f64);
        }
    }
}
