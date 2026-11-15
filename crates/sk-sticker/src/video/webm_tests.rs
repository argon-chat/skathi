use super::*;

fn id_bytes(id: u32) -> Vec<u8> {
    let b = id.to_be_bytes();
    let skip = b.iter().position(|&x| x != 0).unwrap_or(3);
    b[skip..].to_vec()
}

fn size_bytes(v: u64) -> Vec<u8> {
    let len = (1..=8)
        .find(|&l| v < (1u64 << (7 * l)) - 1)
        .expect("fits in 8 bytes");
    let mut out = v.to_be_bytes()[8 - len..].to_vec();
    out[0] |= 0x80 >> (len - 1);
    out
}

pub(crate) fn el(id: u32, data: &[u8]) -> Vec<u8> {
    [id_bytes(id), size_bytes(data.len() as u64), data.to_vec()].concat()
}

fn el_unknown(id: u32, data: &[u8]) -> Vec<u8> {
    [
        id_bytes(id),
        vec![0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        data.to_vec(),
    ]
    .concat()
}

fn el_uint(id: u32, v: u64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let skip = b.iter().position(|&x| x != 0).unwrap_or(7);
    el(id, &b[skip..])
}

pub(crate) fn master(id: u32, children: &[Vec<u8>]) -> Vec<u8> {
    el(id, &children.concat())
}

pub(crate) fn header(doc_type: &str) -> Vec<u8> {
    master(
        EBML,
        &[el_uint(0x4286, 1), el(DOC_TYPE, doc_type.as_bytes())],
    )
}

pub(crate) fn info(scale: u64, duration: Option<f64>) -> Vec<u8> {
    let mut c = vec![el_uint(TIMESTAMP_SCALE, scale)];
    if let Some(d) = duration {
        c.push(el(DURATION, &d.to_be_bytes()));
    }
    master(INFO, &c)
}

pub(crate) fn video_track(number: u64, codec: &str, video: &[Vec<u8>]) -> Vec<u8> {
    master(
        TRACK_ENTRY,
        &[
            el_uint(TRACK_NUMBER, number),
            el_uint(TRACK_TYPE, 1),
            el(CODEC_ID, codec.as_bytes()),
            master(VIDEO, video),
        ],
    )
}

fn audio_track(number: u64) -> Vec<u8> {
    master(
        TRACK_ENTRY,
        &[
            el_uint(TRACK_NUMBER, number),
            el_uint(TRACK_TYPE, 2),
            el(CODEC_ID, b"A_OPUS"),
        ],
    )
}

pub(crate) fn size(w: u64, h: u64) -> Vec<Vec<u8>> {
    vec![el_uint(PIXEL_WIDTH, w), el_uint(PIXEL_HEIGHT, h)]
}

fn block_body(track: u64, rel: i16, flags: u8, payload: &[u8]) -> Vec<u8> {
    [
        size_bytes(track),
        rel.to_be_bytes().to_vec(),
        vec![flags],
        payload.to_vec(),
    ]
    .concat()
}

pub(crate) fn simple_block(track: u64, rel: i16, payload: &[u8]) -> Vec<u8> {
    el(SIMPLE_BLOCK, &block_body(track, rel, 0x80, payload))
}

pub(crate) fn block_group(
    track: u64,
    rel: i16,
    payload: &[u8],
    alpha: Option<&[u8]>,
    duration: Option<u64>,
) -> Vec<u8> {
    let mut c = vec![el(BLOCK, &block_body(track, rel, 0, payload))];
    if let Some(a) = alpha {
        c.push(master(
            BLOCK_ADDITIONS,
            &[master(
                BLOCK_MORE,
                &[el_uint(BLOCK_ADD_ID, 1), el(BLOCK_ADDITIONAL, a)],
            )],
        ));
    }
    if let Some(d) = duration {
        c.push(el_uint(BLOCK_DURATION, d));
    }
    master(BLOCK_GROUP, &c)
}

pub(crate) fn cluster(time: u64, blocks: &[Vec<u8>]) -> Vec<u8> {
    master(
        CLUSTER,
        &[&[el_uint(CLUSTER_TIMESTAMP, time)], blocks].concat(),
    )
}

pub(crate) fn file(segment: &[Vec<u8>]) -> Vec<u8> {
    [header("webm"), master(SEGMENT, segment)].concat()
}

fn sample() -> Vec<u8> {
    let video = [size(64, 48), vec![el_uint(ALPHA_MODE, 1)]].concat();
    file(&[
        el(0xEC, &[0; 5]), // Void
        info(1_000_000, Some(120.0)),
        master(TRACKS, &[audio_track(2), video_track(1, "V_VP9", &video)]),
        cluster(
            0,
            &[
                simple_block(1, 0, b"frame0"),
                block_group(2, 5, b"audio", None, None),
                block_group(1, 40, b"frame1", Some(b"alpha1"), None),
            ],
        ),
        cluster(
            80,
            &[block_group(1, 0, b"frame2", Some(b"alpha2"), Some(40))],
        ),
        master(0x1C53_BB6B, &[el(0xBB, &[0; 8])]), // Cues
    ])
}

#[test]
fn a_file_gives_its_track_and_its_blocks_with_their_alpha() {
    let bytes = sample();
    let webm = demux(&bytes).unwrap();
    assert_eq!(webm.timestamp_scale, 1_000_000);
    assert_eq!(webm.duration, Some(120.0));
    assert_eq!(webm.track.number, 1);
    assert_eq!(webm.track.pixel, Some((64, 48)));
    assert!(webm.track.alpha_mode);
    assert_eq!(webm.track.colour, None);
    type Row<'a> = (i64, &'a [u8], Option<&'a [u8]>, Option<u64>);
    let got: Vec<Row> = webm
        .blocks
        .iter()
        .map(|b| {
            (
                b.time,
                b.data.of(&bytes),
                b.alpha.map(|a| a.of(&bytes)),
                b.duration,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (0, &b"frame0"[..], None, None),
            (40, &b"frame1"[..], Some(&b"alpha1"[..]), None),
            (80, &b"frame2"[..], Some(&b"alpha2"[..]), Some(40)),
        ]
    );
}

#[test]
fn colour_display_size_and_default_duration_are_read() {
    let colour = master(
        COLOUR,
        &[
            el_uint(MATRIX_COEFFICIENTS, 1),
            el_uint(RANGE, 2),
            el_uint(TRANSFER_CHARACTERISTICS, 1),
            el_uint(PRIMARIES, 1),
        ],
    );
    let video = [
        size(100, 50),
        vec![
            el_uint(DISPLAY_WIDTH, 200),
            el_uint(DISPLAY_HEIGHT, 100),
            colour,
        ],
    ]
    .concat();
    let track = master(
        TRACK_ENTRY,
        &[
            el_uint(TRACK_NUMBER, 3),
            el(CODEC_ID, b"V_VP9\0\0"),
            el_uint(DEFAULT_DURATION, 33_333_333),
            master(VIDEO, &video),
        ],
    );
    let bytes = file(&[
        master(TRACKS, &[track]),
        cluster(0, &[simple_block(3, 0, b"x")]),
    ]);
    let webm = demux(&bytes).unwrap();
    let t = &webm.track;
    assert_eq!(
        (t.number, t.pixel, t.display, t.display_unit),
        (3, Some((100, 50)), (200, 100), 0)
    );
    assert_eq!(
        t.colour,
        Some(Colour {
            matrix: Some(1),
            transfer: Some(1),
            primaries: Some(1),
            range: Some(2)
        })
    );
    assert_eq!(
        t.default_duration,
        Some(33_333_333),
        "no TrackType, but a V_ codec: video"
    );
    assert!(!t.alpha_mode);
    assert_eq!(webm.duration, None);
}

#[test]
fn a_live_writers_unknown_sizes_are_read() {
    // MediaRecorder style: Segment and Clusters of unknown size, no Duration.
    let video = size(32, 32);
    let c1 = el_unknown(
        CLUSTER,
        &[
            el_uint(CLUSTER_TIMESTAMP, 0),
            simple_block(1, 0, b"a"),
            simple_block(1, 33, b"b"),
        ]
        .concat(),
    );
    let c2 = el_unknown(
        CLUSTER,
        &[el_uint(CLUSTER_TIMESTAMP, 66), simple_block(1, 0, b"c")].concat(),
    );
    let tags = master(0x1254_C367, &[el(0x7373, &[])]);
    let segment = [
        info(1_000_000, None),
        master(TRACKS, &[video_track(1, "V_VP9", &video)]),
        c1,
        c2,
        tags,
    ]
    .concat();
    let bytes = [header("webm"), el_unknown(SEGMENT, &segment)].concat();
    let webm = demux(&bytes).unwrap();
    let got: Vec<(i64, &[u8])> = webm
        .blocks
        .iter()
        .map(|b| (b.time, b.data.of(&bytes)))
        .collect();
    assert_eq!(got, vec![(0, &b"a"[..]), (33, &b"b"[..]), (66, &b"c"[..])]);
}

#[test]
fn other_tracks_are_skipped_laced_or_not() {
    let laced_audio = el(SIMPLE_BLOCK, &block_body(2, 0, 0x86, b"\x01\x02xx"));
    let bytes = file(&[
        master(
            TRACKS,
            &[audio_track(2), video_track(1, "V_VP9", &size(8, 8))],
        ),
        cluster(0, &[laced_audio, simple_block(1, 0, b"v")]),
    ]);
    assert_eq!(demux(&bytes).unwrap().blocks.len(), 1);
}

#[test]
fn what_is_not_vp9_is_unsupported() {
    let only = |codec: &str| {
        file(&[
            master(TRACKS, &[video_track(1, codec, &size(8, 8))]),
            cluster(0, &[simple_block(1, 0, b"v")]),
        ])
    };
    let unsupported = |bytes: Vec<u8>| match demux(&bytes) {
        Err(VideoError::Unsupported(_)) => {}
        other => panic!("{other:?}"),
    };
    unsupported(only("V_VP8"));
    unsupported(only("V_AV1"));
    unsupported(only("V_MPEG4/ISO/AVC"));
    let audio_only = file(&[
        master(TRACKS, &[audio_track(1)]),
        cluster(0, &[simple_block(1, 0, b"a")]),
    ]);
    unsupported(audio_only);
    let both = file(&[
        master(
            TRACKS,
            &[
                video_track(1, "V_VP8", &size(8, 8)),
                video_track(2, "V_VP9", &size(8, 8)),
            ],
        ),
        cluster(0, &[simple_block(1, 0, b"vp8"), simple_block(2, 0, b"vp9")]),
    ]);
    let webm = demux(&both).unwrap();
    assert_eq!(
        (webm.track.number, webm.blocks[0].data.of(&both)),
        (2, &b"vp9"[..])
    );

    let laced = file(&[
        master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]),
        cluster(0, &[el(SIMPLE_BLOCK, &block_body(1, 0, 0x82, b"\x01xx"))]),
    ]);
    unsupported(laced);
    let encrypted = file(&[
        master(
            TRACKS,
            &[master(
                TRACK_ENTRY,
                &[
                    el_uint(TRACK_NUMBER, 1),
                    el_uint(TRACK_TYPE, 1),
                    el(CODEC_ID, b"V_VP9"),
                    el(CONTENT_ENCODINGS, &[]),
                ],
            )],
        ),
        cluster(0, &[simple_block(1, 0, b"v")]),
    ]);
    unsupported(encrypted);
}

#[test]
fn what_is_not_webm_is_malformed() {
    let malformed = |bytes: &[u8]| match demux(bytes) {
        Err(VideoError::Malformed(_)) => {}
        other => panic!("{other:?}"),
    };
    malformed(b"");
    malformed(b"\x89PNG\r\n\x1a\n");
    malformed(&[header("mp4"), master(SEGMENT, &[])].concat());
    malformed(&header("webm"));
    let early = file(&[
        cluster(0, &[simple_block(1, 0, b"v")]),
        master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]),
    ]);
    malformed(&early);
    let empty = file(&[master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))])]);
    malformed(&empty);
    let empty_block = file(&[
        master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]),
        cluster(0, &[simple_block(1, 0, b"")]),
    ]);
    malformed(&empty_block);
    let zero_scale = file(&[
        info(0, None),
        master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]),
        cluster(0, &[simple_block(1, 0, b"v")]),
    ]);
    malformed(&zero_scale);
    // A matroska DocType plays, as in a browser.
    let mkv = [
        header("matroska"),
        master(
            SEGMENT,
            &[
                master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]),
                cluster(0, &[simple_block(1, 0, b"v")]),
            ],
        ),
    ]
    .concat();
    assert!(demux(&mkv).is_ok());
}

#[test]
fn a_file_cut_inside_what_it_needs_is_malformed_and_one_cut_in_its_trailing_cues_plays() {
    let bytes = sample();
    let cues_start = bytes.len() - master(0x1C53_BB6B, &[el(0xBB, &[0; 8])]).len();
    for n in 0..bytes.len() {
        match demux(&bytes[..n]) {
            Ok(webm) => {
                assert!(
                    n > cues_start,
                    "cut at {n}, before the Cues, and still played"
                );
                assert_eq!(webm.blocks.len(), 3);
            }
            Err(VideoError::Malformed(why)) => {
                assert!(n <= cues_start, "cut at {n} in the Cues: {why}")
            }
            Err(other) => panic!("cut at {n}: {other:?}"),
        }
    }
}

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn check_spans(bytes: &[u8]) {
    if let Ok(webm) = demux(bytes) {
        for b in &webm.blocks {
            let _ = b.data.of(bytes);
            if let Some(a) = b.alpha {
                let _ = a.of(bytes);
            }
        }
    }
}

#[test]
fn damaged_files_are_errors_not_panics() {
    let bytes = sample();
    for bit in 0..bytes.len() * 8 {
        let mut b = bytes.clone();
        b[bit / 8] ^= 1 << (bit % 8);
        check_spans(&b);
    }
    let mut noise = Noise(0x1234_5678_9abc_def1);
    for _ in 0..20_000 {
        let mut b = bytes.clone();
        for _ in 0..1 + noise.next() % 4 {
            let at = (noise.next() % b.len() as u64) as usize;
            b[at] = noise.next() as u8;
        }
        let cut = (noise.next() % (b.len() as u64 + 1)) as usize;
        check_spans(&b[..cut]);
    }
    for _ in 0..2_000 {
        let n = (noise.next() % 64) as usize;
        let junk: Vec<u8> = [header("webm"), (0..n).map(|_| noise.next() as u8).collect()].concat();
        check_spans(&junk);
    }
}

#[test]
fn sizes_the_file_claims_allocate_nothing() {
    let huge = [0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE];
    let claims = |id: u32, inner: &[u8]| [id_bytes(id), huge.to_vec(), inner.to_vec()].concat();
    let tracks = master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]);
    for bytes in [
        [header("webm"), claims(SEGMENT, &[])].concat(),
        file(&[tracks.clone(), claims(CLUSTER, &simple_block(1, 0, b"v"))]),
        file(&[claims(TRACKS, &[])]),
        file(&[tracks.clone(), cluster(0, &[claims(BLOCK_GROUP, &[])])]),
    ] {
        assert!(demux(&bytes).is_err());
    }
}

#[test]
fn a_file_with_more_blocks_than_a_sticker_could_need_is_refused() {
    let block = simple_block(1, 0, b"v");
    let blocks: Vec<Vec<u8>> = (0..MAX_BLOCKS + 1).map(|_| block.clone()).collect();
    let bytes = file(&[
        master(TRACKS, &[video_track(1, "V_VP9", &size(8, 8))]),
        cluster(0, &blocks),
    ]);
    assert!(matches!(demux(&bytes), Err(VideoError::Unsupported(_))));
}
