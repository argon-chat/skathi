//! In-memory sources (`data:` bytes, mapped package bytes) as an `AVURLAsset` on a private URL scheme
//! whose `AVAssetResourceLoader` delegate answers range requests straight from the bytes. No temp
//! file, server or whole-asset copy: mapped bytes are handed over without a copy.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_av_foundation::{
    AVAssetResourceLoader, AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest, AVURLAsset,
};
use objc2_foundation::{NSData, NSObject, NSObjectProtocol, NSString, NSURL};

pub(super) enum Bytes {
    Mapped(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl Bytes {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mapped(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

pub(super) struct Ivars {
    bytes: Bytes,
    uti: &'static str,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Loader does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "SkathiVideoResourceLoader"]
    #[ivars = Ivars]
    pub(super) struct Loader;

    unsafe impl NSObjectProtocol for Loader {}

    unsafe impl AVAssetResourceLoaderDelegate for Loader {
        #[unsafe(method(resourceLoader:shouldWaitForLoadingOfRequestedResource:))]
        fn should_wait(
            &self,
            _loader: &AVAssetResourceLoader,
            request: &AVAssetResourceLoadingRequest,
        ) -> bool {
            self.serve(request);
            true
        }
    }
);

/// A large `data:` source is answered in pieces: each piece is a copy, since AVFoundation may keep
/// an `NSData` past the backend that owns the `Arc`.
const SHARED_CHUNK: usize = 4 << 20;

impl Loader {
    fn serve(&self, request: &AVAssetResourceLoadingRequest) {
        let bytes = self.ivars().bytes.as_slice();
        let len = bytes.len();
        unsafe {
            if let Some(info) = request.contentInformationRequest() {
                info.setContentType(Some(&NSString::from_str(self.ivars().uti)));
                info.setContentLength(len as i64);
                info.setByteRangeAccessSupported(true);
            }
            if let Some(data) = request.dataRequest() {
                let start = (data.requestedOffset().max(0) as usize).min(len);
                let end = if data.requestsAllDataToEndOfResource() {
                    len
                } else {
                    start
                        .saturating_add(data.requestedLength().max(0) as usize)
                        .min(len)
                };
                match &self.ivars().bytes {
                    Bytes::Mapped(mapped) => {
                        let range = &mapped[start..end];
                        // SAFETY: the bytes are `'static` and immutable, so an `NSData` that
                        // borrows them stays valid however long AVFoundation keeps it.
                        let chunk = NSData::dataWithBytesNoCopy_length_freeWhenDone(
                            std::ptr::NonNull::new_unchecked(range.as_ptr() as *mut _),
                            range.len(),
                            false,
                        );
                        data.respondWithData(&chunk);
                    }
                    Bytes::Shared(shared) => {
                        for piece in shared[start..end].chunks(SHARED_CHUNK) {
                            data.respondWithData(&NSData::with_bytes(piece));
                        }
                    }
                }
            }
            request.finishLoading();
        }
    }
}

/// The asset and the delegate serving it. The resource loader holds its delegate weakly, so the
/// backend keeps both for as long as the item plays.
pub(super) struct Served {
    pub asset: Retained<AVURLAsset>,
    pub _loader: Retained<Loader>,
    pub _queue: DispatchRetained<DispatchQueue>,
}

pub(super) fn asset(mime: &str, bytes: Bytes) -> Option<Served> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let uti = uti_for(mime, bytes.as_slice());
    let url = format!(
        "sk-package://{}/asset",
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let url = NSURL::URLWithString(&NSString::from_str(&url))?;
    let loader = Loader::alloc().set_ivars(Ivars { bytes, uti });
    // SAFETY: NSObject's `init` on a freshly allocated instance with its ivars set.
    let loader: Retained<Loader> = unsafe { msg_send![super(loader), init] };
    let queue = DispatchQueue::new("dev.skathi.video.loader", None);
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
    unsafe {
        asset
            .resourceLoader()
            .setDelegate_queue(Some(ProtocolObject::from_ref(&*loader)), Some(&queue));
    }
    Some(Served {
        asset,
        _loader: loader,
        _queue: queue,
    })
}

/// The uniform type the delegate reports: the declared media type, else sniffed from the bytes.
fn uti_for(mime: &str, bytes: &[u8]) -> &'static str {
    let mime = mime.split(';').next().unwrap_or("").trim();
    match mime.to_ascii_lowercase().as_str() {
        "video/mp4" => return "public.mpeg-4",
        "audio/mp4" => return "public.mpeg-4-audio",
        "audio/x-m4a" | "audio/m4a" => return "com.apple.m4a-audio",
        "video/x-m4v" => return "com.apple.m4v-video",
        "video/quicktime" => return "com.apple.quicktime-movie",
        "video/mp2t" => return "public.mpeg-2-transport-stream",
        "audio/mpeg" | "audio/mp3" => return "public.mp3",
        "audio/aac" | "audio/aacp" => return "public.aac-audio",
        "audio/wav" | "audio/x-wav" | "audio/wave" => return "com.microsoft.waveform-audio",
        "audio/flac" | "audio/x-flac" => return "org.xiph.flac",
        "audio/aiff" | "audio/x-aiff" => return "public.aiff-audio",
        _ => {}
    }
    match bytes {
        [_, _, _, _, b'f', b't', b'y', b'p', b'q', b't', b' ', b' ', ..] => {
            "com.apple.quicktime-movie"
        }
        [_, _, _, _, b'f', b't', b'y', b'p', b'M', b'4', b'A', ..] => "com.apple.m4a-audio",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "public.mpeg-4",
        [b'I', b'D', b'3', ..] => "public.mp3",
        [0xff, b, ..] if b & 0xe0 == 0xe0 => "public.mp3",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => {
            "com.microsoft.waveform-audio"
        }
        [b'f', b'L', b'a', b'C', ..] => "org.xiph.flac",
        [b'F', b'O', b'R', b'M', ..] => "public.aiff-audio",
        _ => "public.mpeg-4",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_type_wins_and_an_undeclared_one_is_sniffed() {
        assert_eq!(uti_for("video/mp4; codecs=avc1", b""), "public.mpeg-4");
        assert_eq!(
            uti_for("", b"\0\0\0\x14ftypqt  \0\0\0\0"),
            "com.apple.quicktime-movie"
        );
        assert_eq!(uti_for("", b"\0\0\0\x20ftypisom"), "public.mpeg-4");
        assert_eq!(uti_for("", b"ID3\x04"), "public.mp3");
        assert_eq!(
            uti_for("", b"RIFF\0\0\0\0WAVEfmt "),
            "com.microsoft.waveform-audio"
        );
    }
}
