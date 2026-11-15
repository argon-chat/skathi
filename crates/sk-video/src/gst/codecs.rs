//! `canPlayType` from the GStreamer registry, and the package names that supply a missing element.
//!
//! The registry answers what this machine has right now (hardware decoders included), so a `""` is
//! reliable: a named codec with no decoder, or a container with no demuxer, cannot play.

use super::ffi::{self, Api};
use crate::backend::CanPlay;

/// A media type the registry is asked about, with the package that usually provides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Kind {
    pub caps: &'static str,
    pub name: &'static str,
    pub packages: Packages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Packages {
    Base,
    Good,
    Bad,
    Libav,
}

impl Packages {
    pub fn advice(self) -> &'static str {
        match self {
            Self::Base => {
                "Ubuntu/Debian: `sudo apt install gstreamer1.0-plugins-base`; Fedora: `sudo dnf \
                 install gstreamer1-plugins-base`; Arch: `sudo pacman -S gst-plugins-base`"
            }
            Self::Good => {
                "Ubuntu/Debian: `sudo apt install gstreamer1.0-plugins-good`; Fedora: `sudo dnf \
                 install gstreamer1-plugins-good`; Arch: `sudo pacman -S gst-plugins-good`"
            }
            Self::Bad => {
                "Ubuntu/Debian: `sudo apt install gstreamer1.0-plugins-bad`; Fedora: `sudo dnf \
                 install gstreamer1-plugins-bad-free`; Arch: `sudo pacman -S gst-plugins-bad`"
            }
            Self::Libav => {
                "Ubuntu/Debian: `sudo apt install gstreamer1.0-libav`; Fedora: `sudo dnf install \
                 gstreamer1-plugin-openh264` for H.264, or `gstreamer1-plugin-libav` with RPM \
                 Fusion's `ffmpeg` for H.264, HEVC and AAC; Arch: `sudo pacman -S gst-libav`"
            }
        }
    }
}

pub(super) const H264: Kind = Kind {
    caps: "video/x-h264",
    name: "H.264",
    packages: Packages::Libav,
};
const H265: Kind = Kind {
    caps: "video/x-h265",
    name: "HEVC/H.265",
    packages: Packages::Libav,
};
const AV1: Kind = Kind {
    caps: "video/x-av1",
    name: "AV1",
    packages: Packages::Bad,
};
const VP8: Kind = Kind {
    caps: "video/x-vp8",
    name: "VP8",
    packages: Packages::Good,
};
const VP9: Kind = Kind {
    caps: "video/x-vp9",
    name: "VP9",
    packages: Packages::Good,
};
const THEORA: Kind = Kind {
    caps: "video/x-theora",
    name: "Theora",
    packages: Packages::Base,
};
const MPEG2: Kind = Kind {
    caps: "video/mpeg, mpegversion=(int)2",
    name: "MPEG-2 video",
    packages: Packages::Libav,
};
pub(super) const AAC: Kind = Kind {
    caps: "audio/mpeg, mpegversion=(int)4",
    name: "AAC",
    packages: Packages::Libav,
};
const MP3: Kind = Kind {
    caps: "audio/mpeg, mpegversion=(int)1, layer=(int)3",
    name: "MP3",
    packages: Packages::Good,
};
const OPUS: Kind = Kind {
    caps: "audio/x-opus",
    name: "Opus",
    packages: Packages::Base,
};
const VORBIS: Kind = Kind {
    caps: "audio/x-vorbis",
    name: "Vorbis",
    packages: Packages::Base,
};
const FLAC: Kind = Kind {
    caps: "audio/x-flac",
    name: "FLAC",
    packages: Packages::Good,
};
const AC3: Kind = Kind {
    caps: "audio/x-ac3",
    name: "AC-3",
    packages: Packages::Libav,
};
const EAC3: Kind = Kind {
    caps: "audio/x-eac3",
    name: "E-AC-3",
    packages: Packages::Libav,
};

const CODECS: &[Kind] = &[
    H264, H265, AV1, VP8, VP9, THEORA, MPEG2, AAC, MP3, OPUS, VORBIS, FLAC, AC3, EAC3,
];

fn codec_from_token(token: &str) -> Option<Kind> {
    let t = token.trim().trim_matches('"').to_ascii_lowercase();
    let kind = match t.as_str() {
        _ if t.starts_with("avc1") || t.starts_with("avc3") || t == "h264" => H264,
        _ if t.starts_with("hev1") || t.starts_with("hvc1") || t == "h265" => H265,
        _ if t.starts_with("av01") || t == "av1" => AV1,
        _ if t.starts_with("vp09") || t == "vp9" => VP9,
        _ if t.starts_with("vp08") || t == "vp8" => VP8,
        "theora" => THEORA,
        "mp4a.40.2" | "mp4a.40.5" | "mp4a.40.29" | "mp4a.40" | "aac" => AAC,
        _ if t.starts_with("mp4a.40") => AAC,
        "mp4a.69" | "mp4a.6b" | "mp3" => MP3,
        _ if t.starts_with("mp4v.61") || t.starts_with("mp2v") => MPEG2,
        "opus" | "op" => OPUS,
        "vorbis" => VORBIS,
        "flac" | "fl" => FLAC,
        "ac-3" | "ac3" => AC3,
        "ec-3" | "eac3" => EAC3,
        _ => return None,
    };
    Some(kind)
}

/// The demuxer (or parser, for an elementary stream) caps a MIME container maps to.
fn container_caps(mime: &str) -> Option<(&'static str, Packages)> {
    Some(match mime {
        "video/mp4" | "audio/mp4" | "video/quicktime" | "video/x-m4v" | "audio/x-m4a"
        | "video/3gpp" | "audio/3gpp" => ("video/quicktime", Packages::Good),
        "video/webm" | "audio/webm" => ("video/webm", Packages::Good),
        "video/x-matroska" | "audio/x-matroska" => ("video/x-matroska", Packages::Good),
        "video/ogg" | "audio/ogg" | "application/ogg" => ("application/ogg", Packages::Base),
        "video/mp2t" => ("video/mpegts, systemstream=(boolean)true", Packages::Bad),
        "video/mpeg" => ("video/mpeg, systemstream=(boolean)true", Packages::Libav),
        "video/x-msvideo" | "video/avi" => ("video/x-msvideo", Packages::Good),
        "video/x-flv" => ("video/x-flv", Packages::Good),
        "audio/mpeg" | "audio/mp3" => ("audio/mpeg, mpegversion=(int)1", Packages::Good),
        "audio/aac" | "audio/aacp" => ("audio/mpeg, mpegversion=(int)4", Packages::Libav),
        "audio/wav" | "audio/x-wav" | "audio/wave" => ("audio/x-wav", Packages::Good),
        "audio/flac" | "audio/x-flac" => ("audio/x-flac", Packages::Good),
        _ => return None,
    })
}

/// Whether the registry has an element of `mask` (at a rank playbin would pick) taking `caps`.
pub(super) fn registry_has(api: &Api, mask: u64, caps: &str) -> bool {
    let text = ffi::c(caps);
    // SAFETY: the factory lists are owned here and freed with the list function GStreamer gives.
    unsafe {
        let caps = (api.core.gst_caps_from_string)(text.as_ptr());
        if caps.is_null() {
            return false;
        }
        let all = (api.core.gst_element_factory_list_get_elements)(mask, ffi::RANK_MARGINAL);
        let matching =
            (api.core.gst_element_factory_list_filter)(all, caps, ffi::PAD_SINK, ffi::FALSE);
        let found = !matching.is_null();
        (api.core.gst_plugin_feature_list_free)(matching);
        (api.core.gst_plugin_feature_list_free)(all);
        (api.core.gst_mini_object_unref)(caps);
        found
    }
}

pub(super) fn have_decoder(api: &Api, kind: Kind) -> bool {
    registry_has(api, ffi::FACTORY_DECODER, kind.caps)
}

/// `canPlayType(type)`: `""` when the container has no demuxer or a named codec has no decoder;
/// `"probably"` when the container and every named codec are covered; `"maybe"` otherwise.
pub(super) fn can_play_type(api: &Api, mime_type: &str) -> CanPlay {
    let lower = mime_type.trim().to_ascii_lowercase();
    let mut parts = lower.split(';');
    let container = parts.next().unwrap_or("").trim();
    if !container.starts_with("video/")
        && !container.starts_with("audio/")
        && container != "application/ogg"
    {
        return CanPlay::No;
    }
    let mask = ffi::FACTORY_DEMUXER | ffi::FACTORY_PARSER | ffi::FACTORY_DECODER;
    let known = match container_caps(container) {
        Some((caps, _)) => {
            if !registry_has(api, mask, caps) {
                return CanPlay::No;
            }
            true
        }
        None => {
            if !registry_has(api, mask, container) {
                return CanPlay::No;
            }
            false
        }
    };
    let mut named = 0;
    let mut unknown = false;
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
                Some(kind) if !have_decoder(api, kind) => return CanPlay::No,
                Some(_) => {}
                None => unknown = true,
            }
        }
    }
    if named > 0 && known && !unknown {
        CanPlay::Probably
    } else {
        CanPlay::Maybe
    }
}

/// The codecs a missing-plugin caps string names, for the advisory.
pub(super) fn kind_for_caps(caps: &str) -> Option<Kind> {
    let name = caps.split(',').next().unwrap_or("").trim();
    CODECS.iter().copied().find(|k| {
        let mut want = k.caps.split(',');
        let base = want.next().unwrap_or("").trim();
        base == name && want.all(|field| caps.replace(' ', "").contains(&field.replace(' ', "")))
    })
}

/// "decoders installed / missing" for the common web codecs, from the registry.
pub(super) fn summary(api: &Api) -> (Vec<&'static str>, Vec<Kind>) {
    let mut installed = Vec::new();
    let mut missing = Vec::new();
    for &kind in &[H264, H265, AV1, VP8, VP9, AAC, OPUS] {
        if have_decoder(api, kind) {
            installed.push(kind.name);
        } else {
            missing.push(kind);
        }
    }
    (installed, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_tokens_map_to_gstreamer_caps() {
        assert_eq!(codec_from_token("avc1.42E01E"), Some(H264));
        assert_eq!(codec_from_token("hvc1.1.6.L93.B0"), Some(H265));
        assert_eq!(codec_from_token("av01.0.05M.08"), Some(AV1));
        assert_eq!(codec_from_token("vp09.00.10.08"), Some(VP9));
        assert_eq!(codec_from_token("\"vp8\""), Some(VP8));
        assert_eq!(codec_from_token("mp4a.40.2"), Some(AAC));
        assert_eq!(codec_from_token("opus"), Some(OPUS));
        assert_eq!(codec_from_token("xyz1"), None);
    }

    #[test]
    fn missing_plugin_caps_find_their_package() {
        let h264 = kind_for_caps("video/x-h264, stream-format=(string)avc, alignment=(string)au");
        assert_eq!(h264.map(|k| k.packages), Some(Packages::Libav));
        let aac = kind_for_caps("audio/mpeg, mpegversion=(int)4, stream-format=(string)raw");
        assert_eq!(aac.map(|k| k.name), Some("AAC"));
        let mp3 = kind_for_caps("audio/mpeg, mpegversion=(int)1, layer=(int)3");
        assert_eq!(mp3.map(|k| k.name), Some("MP3"));
        assert_eq!(kind_for_caps("video/x-gst-fourcc-xyz1"), None);
    }
}
