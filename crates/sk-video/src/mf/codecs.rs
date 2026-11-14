//! What this machine can decode, asked of the decoder registry rather than a table or the media
//! engine. `IMFMediaEngine::CanPlayType` is not used: it answers "probably" for nearly every
//! `video/*`, so it can never give a reliable negative; `MFTEnumEx` queries the registry live,
//! including Store codec extensions added later.
//!
//! There is no registry query for "can you demux this container", so the container list is a hint:
//! an unrecognised `video/*` type answers `"maybe"`, never `""` (a Store package makes `video/ogg`
//! play although Ogg is not in Microsoft's inbox list).

use std::collections::HashMap;
use std::sync::OnceLock;

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, MFMediaType_Video, MFTEnumEx, MFVideoFormat_AV1, MFVideoFormat_H264,
    MFVideoFormat_H265, MFVideoFormat_HEVC, MFVideoFormat_MPEG2, MFVideoFormat_Theora,
    MFVideoFormat_VP80, MFVideoFormat_VP90, MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_ALL,
    MFT_REGISTER_TYPE_INFO,
};
use windows::Win32::System::Com::CoTaskMemFree;

use crate::backend::CanPlay;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
    Vp8,
    Vp9,
    Mpeg2,
    Theora,
}

impl Codec {
    pub fn describe(self) -> &'static str {
        match self {
            Self::H264 => "H.264/AVC (inbox on every Windows install)",
            Self::Hevc => "HEVC/H.265 (Microsoft.HEVCVideoExtension)",
            Self::Av1 => "AV1 (Microsoft.AV1VideoExtension)",
            Self::Vp8 => "VP8 (Microsoft.WebMediaExtensions)",
            Self::Vp9 => "VP9 (Microsoft.VP9VideoExtensions)",
            Self::Mpeg2 => "MPEG-2 (Microsoft.MPEG2VideoExtension)",
            Self::Theora => "Theora (Microsoft.WebMediaExtensions)",
        }
    }

    /// HEVC has two registered GUIDs and decoders register under either.
    fn subtypes(self) -> &'static [GUID] {
        match self {
            Self::H264 => &[MFVideoFormat_H264],
            Self::Hevc => &[MFVideoFormat_HEVC, MFVideoFormat_H265],
            Self::Av1 => &[MFVideoFormat_AV1],
            Self::Vp8 => &[MFVideoFormat_VP80],
            Self::Vp9 => &[MFVideoFormat_VP90],
            Self::Mpeg2 => &[MFVideoFormat_MPEG2],
            Self::Theora => &[MFVideoFormat_Theora],
        }
    }

    pub const ALL: &'static [Codec] = &[
        Self::H264,
        Self::Hevc,
        Self::Av1,
        Self::Vp8,
        Self::Vp9,
        Self::Mpeg2,
        Self::Theora,
    ];
}

fn codec_from_token(token: &str) -> Option<Codec> {
    let t = token.trim().trim_matches('"').to_ascii_lowercase();
    if t.starts_with("avc1") || t.starts_with("avc3") || t.starts_with("h264") {
        return Some(Codec::H264);
    }
    if t.starts_with("hev1") || t.starts_with("hvc1") || t.starts_with("h265") {
        return Some(Codec::Hevc);
    }
    if t.starts_with("av01") || t == "av1" {
        return Some(Codec::Av1);
    }
    if t.starts_with("vp09") || t == "vp9" {
        return Some(Codec::Vp9);
    }
    if t.starts_with("vp08") || t == "vp8" {
        return Some(Codec::Vp8);
    }
    if t == "theora" {
        return Some(Codec::Theora);
    }
    None
}

pub fn have_decoder(codec: Codec) -> bool {
    static CACHE: OnceLock<HashMap<Codec, bool>> = OnceLock::new();
    *CACHE
        .get_or_init(|| {
            Codec::ALL
                .iter()
                .map(|&c| (c, c.subtypes().iter().any(|&s| enumerate(s) > 0)))
                .collect()
        })
        .get(&codec)
        .unwrap_or(&false)
}

fn enumerate(subtype: GUID) -> u32 {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype,
    };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    let ok = unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_ALL,
            Some(&input),
            None,
            &mut activates,
            &mut count,
        )
    }
    .is_ok();
    if !ok {
        return 0;
    }
    // `MFTEnumEx` returns a CoTaskMemAlloc'd array of AddRef'd interfaces: release each, then free the array.
    if !activates.is_null() {
        unsafe {
            for i in 0..count as usize {
                let _ = (*activates.add(i)).take();
            }
            CoTaskMemFree(Some(activates as *const _));
        }
    }
    count
}

fn container_known(mime: &str) -> bool {
    matches!(
        mime,
        "video/mp4"
            | "video/quicktime"
            | "video/x-m4v"
            | "video/3gpp"
            | "audio/mp4"
            | "video/x-matroska"
            | "video/webm"
            | "audio/webm"
            | "video/x-msvideo"
            | "video/avi"
            | "video/x-ms-asf"
            | "video/x-ms-wmv"
            | "audio/mpeg"
            | "audio/wav"
            | "audio/x-wav"
            | "audio/aac"
            | "audio/flac"
            // Not in Microsoft's inbox list, but measured working here (a Store package adds the source).
            | "video/ogg"
            | "audio/ogg"
            | "application/ogg"
    )
}

/// `canPlayType(type)` from the registry: `""` when a named video codec has no decoder or the type
/// is not media (the reliable answer); `"maybe"` for a known container with no codecs named, an
/// unrecognised codec token or an unrecognised `video/*` container; `"probably"` for a known
/// container whose named video codecs all have a decoder.
pub fn can_play_type(mime_type: &str) -> CanPlay {
    let lower = mime_type.trim().to_ascii_lowercase();
    let mut parts = lower.split(';');
    let container = parts.next().unwrap_or("").trim();
    let known_container = container_known(container);
    if !known_container && !container.starts_with("video/") && !container.starts_with("audio/") {
        return CanPlay::No;
    }
    let mut named = 0;
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.trim() != "codecs" {
            continue;
        }
        for token in value.trim().trim_matches('"').split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            named += 1;
            match codec_from_token(token) {
                Some(codec) if !have_decoder(codec) => return CanPlay::No,
                Some(_) => {}
                // An audio or unrecognised codec is not a reason to say no, but lowers confidence.
                None => return CanPlay::Maybe,
            }
        }
    }
    if named > 0 && known_container {
        CanPlay::Probably
    } else {
        CanPlay::Maybe
    }
}

pub fn installed_summary() -> (Vec<&'static str>, Vec<&'static str>) {
    let mut installed = Vec::new();
    let mut missing = Vec::new();
    for &codec in Codec::ALL {
        if have_decoder(codec) {
            installed.push(codec.describe());
        } else {
            missing.push(codec.describe());
        }
    }
    (installed, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_tokens_are_matched_by_prefix_because_they_carry_profiles() {
        assert_eq!(codec_from_token("avc1.42E01E"), Some(Codec::H264));
        assert_eq!(codec_from_token("avc1.640028"), Some(Codec::H264));
        assert_eq!(codec_from_token("hev1.1.6.L93.B0"), Some(Codec::Hevc));
        assert_eq!(codec_from_token("hvc1.2.4.L120.B0"), Some(Codec::Hevc));
        assert_eq!(codec_from_token("av01.0.05M.08"), Some(Codec::Av1));
        assert_eq!(codec_from_token("vp09.00.10.08"), Some(Codec::Vp9));
        assert_eq!(codec_from_token("mp4a.40.2"), None);
        assert_eq!(codec_from_token("opus"), None);
    }

    #[test]
    fn only_the_registry_produces_a_no() {
        assert_eq!(can_play_type("application/x-nonsense"), CanPlay::No);
        assert_eq!(can_play_type("text/plain"), CanPlay::No);
        assert_eq!(can_play_type("video/x-flv"), CanPlay::Maybe);
    }

    /// H.264 in MP4 is the inbox baseline; if this is not `probably`, the media stack is missing.
    #[test]
    fn the_inbox_baseline_is_playable() {
        assert_eq!(
            can_play_type("video/mp4; codecs=\"avc1.42E01E, mp4a.40.2\""),
            // `mp4a` is an audio codec we do not judge, so the answer drops from probably to maybe.
            CanPlay::Maybe
        );
        assert_eq!(
            can_play_type("video/mp4; codecs=\"avc1.640028\""),
            CanPlay::Probably
        );
        assert_eq!(can_play_type("video/mp4"), CanPlay::Maybe);
    }
}
