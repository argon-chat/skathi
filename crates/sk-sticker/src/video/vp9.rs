//! What a VP9 packet says about itself, read from its first bits without decoding it: which frames
//! it holds (the superframe index), whether one is shown, the profile, and a keyframe's size and
//! colour. The demuxer uses it to build the frame index and to refuse what libvpx would refuse
//! later, with a better message. Bit layout: the VP9 bitstream specification, sections 6.2
//! (`uncompressed_header`) and B.3 (superframes).

const MAX_FRAMES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frames {
    count: usize,
    ranges: [(usize, usize); MAX_FRAMES],
}

impl Frames {
    pub(crate) fn iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.ranges[..self.count].iter().copied()
    }
}

/// The frames of `packet`: the ranges its superframe index names, or the whole packet when it has
/// no index. `None` for an index whose sizes run past the packet.
pub(crate) fn frames(packet: &[u8]) -> Option<Frames> {
    let mut out = Frames {
        count: 1,
        ranges: [(0, 0); MAX_FRAMES],
    };
    out.ranges[0] = (0, packet.len());
    let Some(&marker) = packet.last() else {
        return Some(Frames { count: 0, ..out });
    };
    if marker & 0xe0 != 0xc0 {
        return Some(out);
    }
    let count = (marker & 7) as usize + 1;
    let mag = ((marker >> 3) & 3) as usize + 1;
    let index_len = 2 + mag * count;
    if packet.len() < index_len || packet[packet.len() - index_len] != marker {
        return Some(out); // a frame that happens to end in a marker-like byte
    }
    let index = &packet[packet.len() - index_len + 1..];
    let data_len = packet.len() - index_len;
    let mut at = 0usize;
    out.count = 0;
    for i in 0..count {
        let size = index[i * mag..(i + 1) * mag]
            .iter()
            .rev()
            .fold(0usize, |s, &b| (s << 8) | b as usize);
        if size > data_len - at {
            return None;
        }
        if size == 0 {
            continue; // libvpx skips an empty frame in an index
        }
        out.ranges[out.count] = (at, at + size);
        out.count += 1;
        at += size;
    }
    Some(out)
}

pub(crate) const CS_RGB: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameHeader {
    pub profile: u8,
    pub shows: bool,
    pub keyframe: bool,
    pub size: Option<(u32, u32)>,
    pub color: Option<(u8, bool)>,
}

/// The header of one frame (not a superframe); `None` when it is not a VP9 frame header.
pub(crate) fn frame_header(frame: &[u8]) -> Option<FrameHeader> {
    let mut b = Bits {
        data: frame,
        pos: 0,
    };
    if b.read(2)? != 2 {
        return None; // frame_marker
    }
    let low = b.read(1)?;
    let high = b.read(1)?;
    let profile = (high << 1 | low) as u8;
    if profile == 3 && b.read(1)? != 0 {
        return None; // reserved_zero
    }
    let mut header = FrameHeader {
        profile,
        shows: false,
        keyframe: false,
        size: None,
        color: None,
    };
    if b.read(1)? == 1 {
        header.shows = true;
        return Some(header);
    }
    let keyframe = b.read(1)? == 0;
    let show_frame = b.read(1)? == 1;
    let error_resilient = b.read(1)? == 1;
    header.keyframe = keyframe;
    header.shows = show_frame;
    if keyframe {
        sync_code(&mut b)?;
        header.color = Some(color_config(&mut b, profile)?);
        header.size = Some(frame_size(&mut b)?);
    } else {
        let intra_only = if show_frame { false } else { b.read(1)? == 1 };
        if !error_resilient {
            b.read(2)?; // reset_frame_context
        }
        if intra_only {
            sync_code(&mut b)?;
            if profile > 0 {
                color_config(&mut b, profile)?;
            }
            b.read(8)?; // refresh_frame_flags
            header.size = Some(frame_size(&mut b)?);
        }
    }
    Some(header)
}

fn sync_code(b: &mut Bits) -> Option<()> {
    (b.read(24)? == 0x49_83_42).then_some(())
}

fn color_config(b: &mut Bits, profile: u8) -> Option<(u8, bool)> {
    if profile >= 2 {
        b.read(1)?; // ten_or_twelve_bit
    }
    let cs = b.read(3)? as u8;
    let full = if cs != CS_RGB {
        let full = b.read(1)? == 1;
        if profile == 1 || profile == 3 {
            b.read(3)?; // subsampling_x, subsampling_y, reserved_zero
        }
        full
    } else {
        if profile == 1 || profile == 3 {
            b.read(1)?; // reserved_zero
        }
        true
    };
    Some((cs, full))
}

fn frame_size(b: &mut Bits) -> Option<(u32, u32)> {
    let w = b.read(16)? + 1;
    let h = b.read(16)? + 1;
    Some((w, h))
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = v << 1 | (byte >> (7 - self.pos % 8)) as u32 & 1;
            self.pos += 1;
        }
        Some(v)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PacketInfo {
    pub profile: u8,
    pub shows: bool,
    pub keyframe: bool,
    pub size: Option<(u32, u32)>,
}

/// Reads every frame header in `packet`. `Err` names what is wrong in words for the advisory.
pub(crate) fn packet_info(packet: &[u8]) -> Result<PacketInfo, &'static str> {
    let frames = frames(packet).ok_or("its superframe index names more bytes than it has")?;
    let mut info = PacketInfo::default();
    let mut first = true;
    for (start, end) in frames.iter() {
        let header = frame_header(&packet[start..end]).ok_or("it is not a VP9 frame")?;
        info.profile = info.profile.max(header.profile);
        info.shows |= header.shows;
        if first {
            info.keyframe = header.keyframe;
            first = false;
        }
        if let Some((w, h)) = header.size {
            let (pw, ph) = info.size.unwrap_or((0, 0));
            info.size = Some((pw.max(w), ph.max(h)));
        }
    }
    if first {
        return Err("it is empty");
    }
    Ok(info)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn keyframe(w: u32, h: u32) -> Vec<u8> {
        let mut bits = BitWriter::default();
        bits.put(2, 2); // frame_marker
        bits.put(1, 0); // profile_low_bit
        bits.put(1, 0); // profile_high_bit
        bits.put(1, 0); // show_existing_frame
        bits.put(1, 0); // frame_type: KEY_FRAME
        bits.put(1, 1); // show_frame
        bits.put(1, 0); // error_resilient_mode
        bits.put(24, 0x49_83_42);
        bits.put(3, 1); // color_space: CS_BT_601
        bits.put(1, 0); // color_range
        bits.put(16, w - 1);
        bits.put(16, h - 1);
        bits.bytes()
    }

    #[derive(Default)]
    struct BitWriter {
        bits: Vec<bool>,
    }

    impl BitWriter {
        fn put(&mut self, n: u32, v: u32) {
            for i in (0..n).rev() {
                self.bits.push(v >> i & 1 == 1);
            }
        }

        fn bytes(&self) -> Vec<u8> {
            self.bits
                .chunks(8)
                .map(|c| {
                    c.iter()
                        .enumerate()
                        .fold(0u8, |b, (i, &on)| b | (on as u8) << (7 - i))
                })
                .collect()
        }
    }

    #[test]
    fn a_keyframe_header_gives_its_profile_size_and_colour() {
        let h = frame_header(&keyframe(100, 60)).unwrap();
        assert_eq!(
            h,
            FrameHeader {
                profile: 0,
                shows: true,
                keyframe: true,
                size: Some((100, 60)),
                color: Some((1, false))
            }
        );
        let mut bad = keyframe(100, 60);
        bad[1] ^= 0x10; // inside the sync code
        assert_eq!(frame_header(&bad), None);
        assert_eq!(frame_header(&keyframe(100, 60)[..4]), None, "cut short");
        assert_eq!(frame_header(&[0x00]), None, "frame marker");
    }

    #[test]
    fn profiles_and_shown_existing_frames_are_read() {
        let h = frame_header(&[0b1010_1000]).unwrap();
        assert_eq!((h.profile, h.shows, h.keyframe), (1, true, false));
        // profile 2 (low=0, high=1), inter frame, hidden: show_existing 0, frame_type 1, show_frame 0.
        let h = frame_header(&[0b1001_0100, 0]).unwrap();
        assert_eq!((h.profile, h.shows, h.keyframe), (2, false, false));
        // profile 3 needs its reserved zero bit.
        assert_eq!(frame_header(&[0b1011_1000]), None);
    }

    #[test]
    fn a_superframe_index_splits_a_packet() {
        let a = keyframe(16, 16);
        let b = [0b1000_0100u8, 0, 0]; // an inter frame, not shown
        let mut packet = [b.as_slice(), a.as_slice()].concat();
        let marker = 0b1100_0001u8;
        packet.extend([marker, b.len() as u8, a.len() as u8, marker]);
        let f = frames(&packet).unwrap();
        assert_eq!(f.iter().collect::<Vec<_>>(), vec![(0, 3), (3, 3 + a.len())]);
        let info = packet_info(&packet).unwrap();
        assert_eq!(
            info,
            PacketInfo {
                profile: 0,
                shows: true,
                keyframe: false,
                size: Some((16, 16))
            }
        );

        // Sizes past the data: refused, as libvpx refuses them.
        let mut bad = packet.clone();
        let n = bad.len();
        bad[n - 2] = 200;
        assert_eq!(frames(&bad), None);
        assert!(packet_info(&bad).is_err());
        // A last byte that looks like a marker without its twin is data.
        let lone = [a.as_slice(), &[marker]].concat();
        assert_eq!(frames(&lone).unwrap().iter().count(), 1);
        assert_eq!(packet_info(&[]), Err("it is empty"));
    }

    #[test]
    fn every_prefix_and_bit_flip_of_a_header_is_read_without_a_panic() {
        let a = keyframe(512, 288);
        let marker = 0b1100_1000u8; // 1 frame, 2-byte sizes
        let packet = [a.as_slice(), &[marker, a.len() as u8, 0, marker]].concat();
        for n in 0..=packet.len() {
            let _ = packet_info(&packet[..n]);
        }
        for bit in 0..packet.len() * 8 {
            let mut p = packet.clone();
            p[bit / 8] ^= 1 << (bit % 8);
            let _ = packet_info(&p);
        }
    }
}
