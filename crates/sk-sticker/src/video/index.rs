//! From a demuxed file to what a decoder walks: the packets in order, which
//! of them show a frame, and when ([`VideoMeta`]).

use super::vp9;
use super::webm::{self, Block, Colour, Span};
use super::{VideoError, VideoMeta};

/// The largest frame side libvpx is built to decode (`--size-limit`).
pub(crate) const MAX_SIDE: u64 = 4096;

/// A frame interval when the file gives none (one frame, no durations).
const FALLBACK_FRAME_MS: u32 = 33;

/// One block: its colour packet, its alpha packet, and whether it shows a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Packet {
    pub color: Span,
    pub alpha: Option<Span>,
    pub shows: bool,
}

pub(crate) struct Index {
    pub meta: VideoMeta,
    pub packets: Box<[Packet]>,
    pub colour: Option<Colour>,
}

fn malformed(why: impl Into<String>) -> VideoError {
    VideoError::Malformed(why.into())
}

fn unsupported(why: impl Into<String>) -> VideoError {
    VideoError::Unsupported(why.into())
}

fn check(info: &vp9::PacketInfo, stream: &str) -> Result<(), VideoError> {
    if info.profile != 0 {
        return Err(unsupported(format!(
            "its {stream}video is VP9 profile {}; video stickers are profile 0 (8-bit, 4:2:0)",
            info.profile
        )));
    }
    if let Some((w, h)) = info.size {
        if w as u64 > MAX_SIDE || h as u64 > MAX_SIDE {
            return Err(unsupported(format!("its {stream}frames are {w}×{h}, over the {MAX_SIDE} px a side a sticker may have")));
        }
    }
    Ok(())
}

pub(crate) fn index(bytes: &[u8]) -> Result<Index, VideoError> {
    let webm = webm::demux(bytes)?;
    // The alpha stream plays when the track says so and its blocks carry it
    // (a player ignores BlockAdditions otherwise).
    let has_alpha = webm.track.alpha_mode && webm.blocks.iter().any(|b| b.alpha.is_some());
    let mut packets = Vec::with_capacity(webm.blocks.len());
    let mut shown: Vec<&Block> = Vec::with_capacity(webm.blocks.len());
    let mut keyframe_size = None;
    for (i, block) in webm.blocks.iter().enumerate() {
        let info = vp9::packet_info(block.data.of(bytes)).map_err(|why| malformed(format!("video packet {i}: {why}")))?;
        check(&info, "")?;
        if i == 0 && !info.keyframe {
            return Err(malformed("the video does not start with a keyframe"));
        }
        keyframe_size = keyframe_size.or(info.size);
        let alpha = block.alpha.filter(|_| has_alpha);
        if let Some(a) = alpha {
            let info = vp9::packet_info(a.of(bytes)).map_err(|why| malformed(format!("the alpha of video packet {i}: {why}")))?;
            check(&info, "alpha ")?;
        }
        if info.shows {
            shown.push(block);
        }
        packets.push(Packet { color: block.data, alpha, shows: info.shows });
    }
    if shown.is_empty() {
        return Err(malformed("the video track shows no frame"));
    }

    let scale = webm.timestamp_scale as i128;
    let first = shown[0].time as i128;
    let ms = |t: i64| ((t as i128 - first) * scale + 500_000).div_euclid(1_000_000);
    let mut pts_ms = Vec::with_capacity(shown.len());
    let mut prev = 0u32;
    for block in &shown {
        prev = ms(block.time).clamp(prev as i128, u32::MAX as i128) as u32;
        pts_ms.push(prev);
    }
    let last = prev;
    let from_info = webm.duration.map(|d| (d * webm.timestamp_scale as f64 - first as f64 * webm.timestamp_scale as f64) / 1e6);
    let duration_ms = match from_info.map(f64::round) {
        Some(d) if d > last as f64 && d <= u32::MAX as f64 => d as u32,
        _ => {
            let last_block = shown[shown.len() - 1];
            let gap = last_block
                .duration
                .map(|d| (d as i128 * scale + 500_000) / 1_000_000)
                .or(webm.track.default_duration.map(|ns| (ns as i128 + 500_000) / 1_000_000))
                .or((pts_ms.len() >= 2).then(|| (last - pts_ms[pts_ms.len() - 2]) as i128))
                .filter(|&g| g > 0)
                .unwrap_or(FALLBACK_FRAME_MS as i128);
            (last as i128 + gap).min(u32::MAX as i128) as u32
        }
    };

    let (pw, ph) = webm
        .track
        .pixel
        .or(keyframe_size.map(|(w, h)| (w as u64, h as u64)))
        .ok_or_else(|| malformed("its video track has no size"))?;
    if pw > MAX_SIDE || ph > MAX_SIDE {
        return Err(unsupported(format!("its video is {pw}×{ph}, over the {MAX_SIDE} px a side a sticker may have")));
    }
    let (dw, dh) = webm.track.display;
    let (width, height) = match webm.track.display_unit {
        0 => (if dw > 0 { dw } else { pw }, if dh > 0 { dh } else { ph }),
        3 if dw > 0 && dh > 0 => ((ph * dw + dh / 2) / dh, ph),
        _ => (pw, ph),
    };
    // A display size past what a frame may be is not believed.
    let (width, height) = if (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height) { (width, height) } else { (pw, ph) };

    Ok(Index {
        meta: VideoMeta { width: width as u32, height: height as u32, pts_ms: pts_ms.into(), duration_ms, has_alpha },
        packets: packets.into_boxed_slice(),
        colour: webm.track.colour,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::vp9::tests::keyframe;
    use crate::video::webm::tests::*;

    const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
    const DEFAULT_DURATION: u32 = 0x23_E383;
    const DISPLAY_WIDTH: u32 = 0x54B0;
    const DISPLAY_HEIGHT: u32 = 0x54BA;
    const DISPLAY_UNIT: u32 = 0x54B2;
    const ALPHA_MODE: u32 = 0x53C0;
    const TRACKS: u32 = 0x1654_AE6B;
    const INFO: u32 = 0x1549_A966;

    fn uint(id: u32, v: u64) -> Vec<u8> {
        el(id, &v.to_be_bytes())
    }

    fn inter(shown: bool) -> Vec<u8> {
        // frame_marker 10, profile 00, show_existing 0, frame_type 1, show_frame, error_resilient 0
        vec![0b1000_0100 | (shown as u8) << 1, 0, 0]
    }

    fn with(track_video: &[Vec<u8>], segment_head: &[Vec<u8>], blocks: &[Vec<u8>]) -> Vec<u8> {
        let tracks = master(TRACKS, &[video_track(1, "V_VP9", track_video)]);
        file(&[segment_head.to_vec(), vec![tracks, cluster(0, blocks)]].concat())
    }

    #[test]
    fn hidden_packets_have_no_time_and_the_loop_ends_at_the_duration() {
        let k = keyframe(64, 48);
        let bytes = with(
            &size(64, 48),
            &[info(1_000_000, Some(100.0))],
            &[simple_block(1, 0, &k), simple_block(1, 20, &inter(false)), simple_block(1, 33, &inter(true)), simple_block(1, 67, &inter(true))],
        );
        let index = index(&bytes).unwrap();
        assert_eq!(&*index.meta.pts_ms, &[0, 33, 67]);
        assert_eq!(index.meta.duration_ms, 100);
        assert_eq!(index.packets.iter().map(|p| p.shows).collect::<Vec<_>>(), vec![true, false, true, true]);
        assert_eq!((index.meta.width, index.meta.height, index.meta.has_alpha), (64, 48, false));
    }

    #[test]
    fn without_a_duration_the_last_frame_lasts_its_block_its_track_or_the_gap_before_it() {
        let k = keyframe(8, 8);
        let blocks = |last: Vec<u8>| vec![simple_block(1, 0, &k), simple_block(1, 40, &inter(true)), last];
        let dur = |video: &[Vec<u8>], blocks: Vec<Vec<u8>>| index(&with(video, &[], &blocks)).unwrap().meta.duration_ms;
        assert_eq!(dur(&size(8, 8), blocks(simple_block(1, 100, &inter(true)))), 160, "the gap before it");
        assert_eq!(dur(&size(8, 8), blocks(block_group(1, 100, &inter(true), None, Some(25)))), 125, "its BlockDuration");
        let track = master(0xAE, &[uint(0xD7, 1), el(0x86, b"V_VP9"), uint(DEFAULT_DURATION, 16_666_667), master(0xE0, &size(8, 8))]);
        let bytes = file(&[master(TRACKS, &[track]), cluster(0, &blocks(simple_block(1, 100, &inter(true))))]);
        assert_eq!(index(&bytes).unwrap().meta.duration_ms, 117, "the track's DefaultDuration");
        assert_eq!(dur(&size(8, 8), vec![simple_block(1, 0, &k)]), FALLBACK_FRAME_MS, "one frame, nothing said");
        // An Info Duration short of the last frame is not believed.
        let short = with(&size(8, 8), &[info(1_000_000, Some(50.0))], &blocks(simple_block(1, 100, &inter(true))));
        assert_eq!(index(&short).unwrap().meta.duration_ms, 160);
    }

    #[test]
    fn times_are_from_the_first_frame_in_any_timestamp_scale() {
        // 10 µs units; the first block at 1000 units (10 ms).
        let k = keyframe(8, 8);
        let head = master(INFO, &[uint(TIMESTAMP_SCALE, 10_000), el(0x4489, &(1000.0f64 + 5000.0).to_be_bytes())]);
        let tracks = master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]);
        let bytes = file(&[head, tracks, cluster(1000, &[simple_block(1, 0, &k), simple_block(1, 1667, &inter(true)), simple_block(1, 3333, &inter(true))])]);
        let index = index(&bytes).unwrap();
        assert_eq!(&*index.meta.pts_ms, &[0, 17, 33]);
        assert_eq!(index.meta.duration_ms, 50);
    }

    #[test]
    fn the_display_size_is_the_meta_size() {
        let k = keyframe(100, 50);
        let size_of = |video: Vec<Vec<u8>>| {
            let m = index(&with(&video, &[], &[simple_block(1, 0, &k)])).unwrap().meta;
            (m.width, m.height)
        };
        assert_eq!(size_of(size(100, 50)), (100, 50));
        assert_eq!(size_of([size(100, 50), vec![uint(DISPLAY_WIDTH, 200)]].concat()), (200, 50));
        assert_eq!(size_of([size(100, 50), vec![uint(DISPLAY_WIDTH, 16), uint(DISPLAY_HEIGHT, 9), uint(DISPLAY_UNIT, 3)]].concat()), (89, 50));
        assert_eq!(size_of([size(100, 50), vec![uint(DISPLAY_WIDTH, 99_999)]].concat()), (100, 50), "not believed");
        assert_eq!(size_of(vec![]), (100, 50), "no PixelWidth: the keyframe's");
    }

    #[test]
    fn what_libvpx_would_refuse_is_refused_first_with_a_reason() {
        let err = |video: &[Vec<u8>], blocks: &[Vec<u8>]| index(&with(video, &[], blocks)).err().unwrap();
        let k = keyframe(8, 8);
        assert!(matches!(err(&size(8, 8), &[simple_block(1, 0, &inter(true))]), VideoError::Malformed(w) if w.contains("keyframe")));
        assert!(matches!(err(&size(8, 8), &[simple_block(1, 0, b"\x00junk")]), VideoError::Malformed(w) if w.contains("not a VP9 frame")));
        let mut p1 = k.clone();
        p1[0] |= 0b0010_0000; // profile_low_bit: profile 1
        assert!(matches!(err(&size(8, 8), &[simple_block(1, 0, &p1)]), VideoError::Unsupported(w) if w.contains("profile 1")));
        assert!(matches!(err(&size(8, 8), &[simple_block(1, 0, &keyframe(5000, 8))]), VideoError::Unsupported(w) if w.contains("5000×8")));
        assert!(matches!(err(&size(5000, 8), &[simple_block(1, 0, &k)]), VideoError::Unsupported(w) if w.contains("5000×8")));
        assert!(matches!(err(&size(8, 8), &[simple_block(1, 0, &inter(false))]), VideoError::Malformed(_)));
        let alpha_p1 = [size(8, 8), vec![uint(ALPHA_MODE, 1)]].concat();
        assert!(matches!(err(&alpha_p1, &[block_group(1, 0, &k, Some(&p1), None)]), VideoError::Unsupported(w) if w.contains("alpha video is VP9 profile 1")));
    }

    #[test]
    fn alpha_plays_when_the_track_says_so_and_its_blocks_carry_it() {
        let k = keyframe(8, 8);
        let alpha_mode = [size(8, 8), vec![uint(ALPHA_MODE, 1)]].concat();
        let has = |video: &[Vec<u8>], blocks: &[Vec<u8>]| {
            let i = index(&with(video, &[], blocks)).unwrap();
            (i.meta.has_alpha, i.packets.iter().filter(|p| p.alpha.is_some()).count())
        };
        assert_eq!(has(&alpha_mode, &[block_group(1, 0, &k, Some(&k), None)]), (true, 1));
        assert_eq!(has(&size(8, 8), &[block_group(1, 0, &k, Some(&k), None)]), (false, 0), "no AlphaMode");
        assert_eq!(has(&alpha_mode, &[simple_block(1, 0, &k)]), (false, 0), "no BlockAdditional");
    }

    /// `SK_STICKER_SAMPLES=<dir> cargo test -p sk-sticker samples -- --ignored --nocapture`: indexes
    /// every `.webm` in a directory (real stickers, not checked in) and prints what it found.
    #[test]
    #[ignore]
    fn samples() {
        let dir = std::env::var("SK_STICKER_SAMPLES").expect("SK_STICKER_SAMPLES=<dir of .webm>");
        let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        paths.sort();
        for path in paths.iter().filter(|p| p.extension().is_some_and(|e| e == "webm")) {
            let bytes = std::fs::read(path).unwrap();
            match index(&bytes) {
                Ok(i) => {
                    let m = &i.meta;
                    let hidden = i.packets.iter().filter(|p| !p.shows).count();
                    println!(
                        "{}: {}x{} {} frames ({hidden} hidden packets) {} ms {:.2} fps alpha {} colour {:?}",
                        path.display(),
                        m.width,
                        m.height,
                        m.frame_count(),
                        m.duration_ms,
                        m.fps(),
                        m.has_alpha,
                        i.colour
                    );
                }
                Err(e) => println!("{}: {e}", path.display()),
            }
        }
    }

    #[test]
    fn the_checked_in_clips_index() {
        let alpha = index(include_bytes!("testdata/alpha.webm")).unwrap();
        let m = &alpha.meta;
        assert_eq!((m.width, m.height, m.frame_count(), m.duration_ms, m.has_alpha), (41, 29, 48, 1600, true));
        assert_eq!(&m.pts_ms[..4], &[0, 33, 67, 100]);
        assert!(alpha.packets.iter().all(|p| p.alpha.is_some()));
        let sf = index(include_bytes!("testdata/superframe.webm")).unwrap();
        let m = &sf.meta;
        assert_eq!((m.width, m.height, m.frame_count(), m.duration_ms, m.has_alpha), (72, 56, 48, 1600, false));
        assert!((m.fps() - 30.0).abs() < 0.01);
    }
}
