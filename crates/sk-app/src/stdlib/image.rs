//! `skathi:image`: image files decoded and encoded on worker threads, without a canvas. Decoded
//! pixels are straight RGBA8 in native memory that JS sees through an external `ArrayBuffer`.
//!
//! The file is untrusted: dimensions are checked before anything the size of the image is
//! allocated, and an animation is walked under a budget of composited pixels.

use std::ffi::c_void;
use std::io::Cursor;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, LazyLock, Mutex};

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{
    AnimationDecoder, ExtendedColorType, ImageDecoder as _, ImageEncoder as _, ImageFormat, Limits,
};
use sk_runtime::host;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect, RjsAbInfo};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::image as web_image;
use sk_runtime::redjs::webapi::util::*;

use super::memory::{self, Region};
use super::support::{self, *};

const MAX_SIDE: u32 = 16384;
const MAX_PIXELS: u64 = 1 << 26;
const MAX_ENCODED: usize = 256 << 20;
/// Frames times the pixels of the canvas, for decoding or counting the frames of an animation.
const ANIMATION_BUDGET: u64 = 1 << 32;

const BUFFER: &str = "skathi.buffer";
const VIEW: &str = "skathi.view";
const READER: &str = "skathi.reader";
const ON_CHUNK: &str = "skathi.onChunk";
const ON_FAIL: &str = "skathi.onFail";
const TRACKS: &str = "skathi.tracks";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    Raster(ImageFormat),
    Svg,
}

use Codec::{Raster, Svg};

/// The first type of a codec is the one `ImageDecoder.type` reports; the others are aliases.
const DECODABLE: &[(&str, Codec)] = &[
    ("image/png", Raster(ImageFormat::Png)),
    ("image/jpeg", Raster(ImageFormat::Jpeg)),
    ("image/webp", Raster(ImageFormat::WebP)),
    ("image/gif", Raster(ImageFormat::Gif)),
    ("image/bmp", Raster(ImageFormat::Bmp)),
    ("image/x-icon", Raster(ImageFormat::Ico)),
    ("image/vnd.microsoft.icon", Raster(ImageFormat::Ico)),
    ("image/tiff", Raster(ImageFormat::Tiff)),
    ("image/x-portable-anymap", Raster(ImageFormat::Pnm)),
    ("image/x-portable-bitmap", Raster(ImageFormat::Pnm)),
    ("image/x-portable-graymap", Raster(ImageFormat::Pnm)),
    ("image/x-portable-pixmap", Raster(ImageFormat::Pnm)),
    ("image/svg+xml", Svg),
    ("image/qoi", Raster(ImageFormat::Qoi)),
    ("image/x-tga", Raster(ImageFormat::Tga)),
    ("image/x-targa", Raster(ImageFormat::Tga)),
    ("image/vnd.radiance", Raster(ImageFormat::Hdr)),
    ("image/x-farbfeld", Raster(ImageFormat::Farbfeld)),
    ("image/vnd-ms.dds", Raster(ImageFormat::Dds)),
    ("image/vnd.ms-dds", Raster(ImageFormat::Dds)),
];
const ENCODABLE: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/x-icon",
    "image/bmp",
    "image/qoi",
    "image/tiff",
    "image/x-tga",
    "image/x-portable-anymap",
    "image/x-farbfeld",
    "image/vnd.radiance",
];
const ICO_SIDE: u32 = 256;

#[derive(Debug)]
enum Failure {
    Encoding(String),
    Range(String),
}

impl From<image::ImageError> for Failure {
    fn from(e: image::ImageError) -> Failure {
        Failure::Encoding(e.to_string())
    }
}

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn codec_of(mime: &str) -> Option<Codec> {
    let mime = mime.trim().to_ascii_lowercase();
    DECODABLE.iter().find(|(m, _)| *m == mime).map(|(_, c)| *c)
}

fn canonical(codec: Codec) -> &'static str {
    DECODABLE
        .iter()
        .find(|(_, c)| *c == codec)
        .map_or("", |(m, _)| *m)
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits
}

fn admit((width, height): (u32, u32)) -> Result<(), Failure> {
    if width == 0 || height == 0 {
        return Err(Failure::Encoding("the image has no pixels".into()));
    }
    if width > MAX_SIDE || height > MAX_SIDE || width as u64 * height as u64 > MAX_PIXELS {
        return Err(Failure::Encoding(format!(
            "the image is {width}x{height}; images of up to {MAX_SIDE} pixels a side and {MAX_PIXELS} pixels in all are decoded"
        )));
    }
    Ok(())
}

/// The frames of an animated file and the size of its canvas; `None` for a still image.
fn animation(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<Option<(image::Frames<'_>, (u32, u32))>, Failure> {
    let cursor = Cursor::new(bytes);
    Ok(match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(cursor)?;
            let canvas = decoder.dimensions();
            admit(canvas)?;
            decoder.set_limits(limits())?;
            Some((decoder.into_frames(), canvas))
        }
        ImageFormat::Png => {
            let mut decoder = PngDecoder::new(cursor)?;
            if !decoder.is_apng()? {
                return Ok(None);
            }
            let canvas = decoder.dimensions();
            admit(canvas)?;
            decoder.set_limits(limits())?;
            Some((decoder.apng()?.into_frames(), canvas))
        }
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(cursor)?;
            if !decoder.has_animation() {
                return Ok(None);
            }
            let canvas = decoder.dimensions();
            admit(canvas)?;
            Some((decoder.into_frames(), canvas))
        }
        _ => None,
    })
}

enum Walk {
    Frame(Pixels),
    Count(usize),
}

/// Frame `stop_at`, or the number of frames when there is no such frame or none is asked for.
fn walk(
    frames: image::Frames<'_>,
    (width, height): (u32, u32),
    stop_at: Option<usize>,
) -> Result<Walk, Failure> {
    let per_frame = width as u64 * height as u64;
    let mut count = 0usize;
    for frame in frames {
        if (count as u64 + 1) * per_frame > ANIMATION_BUDGET {
            return Err(Failure::Encoding(format!(
                "the animation has more than {count} frames of {width}x{height}, more than is decoded"
            )));
        }
        let frame = frame?;
        if stop_at == Some(count) {
            let buffer = frame.into_buffer();
            return Ok(Walk::Frame(Pixels {
                width: buffer.width(),
                height: buffer.height(),
                rgba: buffer.into_raw(),
            }));
        }
        count += 1;
    }
    Ok(Walk::Count(count))
}

fn no_frame(index: usize, count: usize) -> Failure {
    Failure::Range(format!(
        "frame {index} does not exist; the image has {count}"
    ))
}

fn still(bytes: &[u8], format: ImageFormat, index: usize) -> Result<Pixels, Failure> {
    if index != 0 {
        return Err(no_frame(index, 1));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let mut decoder = reader.into_decoder()?;
    admit(decoder.dimensions())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.into_rgba8();
    Ok(Pixels {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// The renderer `<img>` draws an SVG with, so text finds the same fonts.
static SVG_RENDERER: LazyLock<gpui::SvgRenderer> =
    LazyLock::new(|| gpui::SvgRenderer::new(Arc::new(crate::payload::GpuiAssets)));

/// The SVG's own size in pixels, read as `<img>` reads it.
fn svg_size(bytes: &[u8]) -> Result<(u32, u32), Failure> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| Failure::Encoding(format!("the SVG could not be read: {e}")))?;
    let size = tree.size();
    let size = (size.width().round() as u32, size.height().round() as u32);
    admit(size)?;
    Ok(size)
}

fn svg(bytes: &[u8], index: usize, max_side: Option<u32>) -> Result<Pixels, Failure> {
    if index != 0 {
        return Err(no_frame(index, 1));
    }
    let (width, height) = svg_size(bytes)?;
    let (width, height) = max_side.map_or((width, height), |side| fitted(width, height, side));
    let failed = |e: usvg::Error| Failure::Encoding(format!("the SVG could not be drawn: {e}"));
    let parsed = SVG_RENDERER.parse_svg(bytes).map_err(failed)?;
    let size = gpui::size(
        gpui::DevicePixels(width as i32),
        gpui::DevicePixels(height as i32),
    );
    let image = SVG_RENDERER
        .render_parsed(&parsed, gpui::SvgSize::ExactSize(size))
        .map_err(failed)?;
    let drawn = image.size(0);
    let mut rgba = image.as_bytes(0).unwrap_or_default().to_vec();
    // gpui hands back straight BGRA.
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(Pixels {
        width: drawn.width.0 as u32,
        height: drawn.height.0 as u32,
        rgba,
    })
}

/// Frame `index` of the file, upright by its EXIF orientation, no larger than `max_side` a side.
fn decode(
    bytes: &[u8],
    codec: Codec,
    index: usize,
    max_side: Option<u32>,
) -> Result<Pixels, Failure> {
    let format = match codec {
        Raster(format) => format,
        Svg => return svg(bytes, index, max_side),
    };
    let pixels = match animation(bytes, format)? {
        Some((frames, canvas)) => match walk(frames, canvas, Some(index))? {
            Walk::Frame(pixels) => pixels,
            Walk::Count(count) => return Err(no_frame(index, count)),
        },
        None => still(bytes, format, index)?,
    };
    Ok(match max_side {
        Some(side) => fit(pixels, side),
        None => pixels,
    })
}

/// The number of frames, and whether the file is an animation of more than one.
fn probe(bytes: &[u8], codec: Codec) -> Result<(usize, bool), Failure> {
    let format = match codec {
        Raster(format) => format,
        Svg => return svg_size(bytes).map(|_| (1, false)),
    };
    match animation(bytes, format)? {
        Some((frames, canvas)) => match walk(frames, canvas, None)? {
            Walk::Count(0) => Err(Failure::Encoding("the animation has no frames".into())),
            Walk::Count(count) => Ok((count, count > 1)),
            Walk::Frame(_) => unreachable!("no frame was asked for"),
        },
        None => {
            let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(limits());
            admit(reader.into_decoder()?.dimensions())?;
            Ok((1, false))
        }
    }
}

/// `width` x `height` made no larger than `side` a side, never enlarged.
fn fitted(width: u32, height: u32, side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= side {
        return (width, height);
    }
    let scale = |n: u32| ((n as f64 * side as f64 / longest as f64).round() as u32).clamp(1, side);
    (scale(width), scale(height))
}

fn fit(pixels: Pixels, side: u32) -> Pixels {
    let (width, height) = fitted(pixels.width, pixels.height, side);
    if (width, height) == (pixels.width, pixels.height) {
        return pixels;
    }
    Pixels {
        rgba: sk_resample::downscale(
            &pixels.rgba,
            pixels.width,
            pixels.height,
            width,
            height,
            false,
        ),
        width,
        height,
    }
}

fn encode(
    rgba: &[u8],
    width: u32,
    height: u32,
    mime: &'static str,
    quality: Option<f64>,
) -> Result<Vec<u8>, Failure> {
    match codec_of(mime) {
        Some(Raster(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)) => {}
        Some(Raster(format)) => return encode_still(rgba, width, height, format),
        _ => {
            return Err(Failure::Encoding(format!(
                "the image could not be encoded as {mime}"
            )))
        }
    }
    let quality = if mime == "image/jpeg" { quality } else { None };
    let (made, bytes) =
        sk_runtime::redjs::webapi::canvas2d::encode::encode(rgba, width, height, mime, quality);
    if made != mime {
        return Err(Failure::Encoding(format!(
            "the image could not be encoded as {mime}"
        )));
    }
    Ok(bytes)
}

/// One image in a type the canvas does not write, from straight RGBA8.
fn encode_still(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: ImageFormat,
) -> Result<Vec<u8>, Failure> {
    let mut out = Vec::new();
    let rgba8 = ExtendedColorType::Rgba8;
    match format {
        ImageFormat::Gif => {
            image::codecs::gif::GifEncoder::new(&mut out).encode(rgba, width, height, rgba8)?
        }
        ImageFormat::Ico => {
            if width > ICO_SIDE || height > ICO_SIDE {
                return Err(Failure::Encoding(format!(
                    "the image is {width}x{height}; image/x-icon holds images of up to {ICO_SIDE} pixels a side, so resize it first"
                )));
            }
            image::codecs::ico::IcoEncoder::new(&mut out).write_image(rgba, width, height, rgba8)?
        }
        ImageFormat::Bmp => {
            image::codecs::bmp::BmpEncoder::new(&mut out).write_image(rgba, width, height, rgba8)?
        }
        ImageFormat::Qoi => {
            image::codecs::qoi::QoiEncoder::new(&mut out).write_image(rgba, width, height, rgba8)?
        }
        ImageFormat::Tiff => image::codecs::tiff::TiffEncoder::new(Cursor::new(&mut out))
            .write_image(rgba, width, height, rgba8)?,
        ImageFormat::Tga => {
            image::codecs::tga::TgaEncoder::new(&mut out).write_image(rgba, width, height, rgba8)?
        }
        ImageFormat::Pnm => image::codecs::pnm::PnmEncoder::new(&mut out)
            .with_subtype(image::codecs::pnm::PnmSubtype::ArbitraryMap)
            .write_image(rgba, width, height, rgba8)?,
        ImageFormat::Farbfeld => {
            let wide: Vec<u8> = rgba
                .iter()
                .flat_map(|&c| (c as u16 * 257).to_ne_bytes())
                .collect();
            image::codecs::farbfeld::FarbfeldEncoder::new(&mut out).write_image(
                &wide,
                width,
                height,
                ExtendedColorType::Rgba16,
            )?
        }
        ImageFormat::Hdr => {
            let rgb: Vec<u8> = rgba
                .chunks_exact(4)
                .flat_map(|p| &p[..3])
                .flat_map(|&c| (c as f32 / 255.0).to_ne_bytes())
                .collect();
            image::codecs::hdr::HdrEncoder::new(&mut out).write_image(
                &rgb,
                width,
                height,
                ExtendedColorType::Rgb32F,
            )?
        }
        _ => {
            return Err(Failure::Encoding(format!(
                "the image could not be encoded as {format:?}"
            )))
        }
    }
    Ok(out)
}

type Finish = Box<dyn FnOnce() + Send>;

static FINISHED: Mutex<Vec<Finish>> = Mutex::new(Vec::new());

/// Runs `work` on the native-module task pool; `done` runs on the JS thread at the next pump.
fn on_worker<R: Send + 'static>(
    g: *mut c_void,
    work: impl FnOnce() -> Result<R, Failure> + Send + 'static,
    done: impl FnOnce(*mut c_void, Result<R, Failure>) + Send + 'static,
) {
    let g = g as usize;
    crate::embed::pool_run(Box::new(move || {
        let result = std::panic::catch_unwind(AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err(Failure::Encoding("the image could not be processed".into())));
        FINISHED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(move || done(g as *mut c_void, result)));
        sk_runtime::frames::wake();
        crate::background::prompt();
    }));
}

/// Settles the promises of the work that finished since the last frame.
pub(crate) fn pump() {
    let finished = std::mem::take(&mut *FINISHED.lock().unwrap_or_else(|e| e.into_inner()));
    for finish in finished {
        finish();
    }
}

unsafe fn failure(g: *mut c_void, f: &Failure) -> u64 {
    match f {
        Failure::Encoding(m) => host::error(g, "EncodingError", m),
        Failure::Range(m) => host::error(g, "RangeError", m),
    }
}

unsafe fn pending_exception(g: *mut c_void) -> u64 {
    let mut error = UNDEF;
    glue::rjs_exception_pending(g, &mut error);
    error
}

/// The promise `run` returns, or one rejected with what it threw.
unsafe fn promise_of(g: *mut c_void, run: impl FnOnce() -> Js<u64>) -> u64 {
    match run() {
        Ok(promise) => promise,
        Err(Thrown) => host::rejected_with(g, pending_exception(g)),
    }
}

unsafe fn dom<T>(g: *mut c_void, name: &str, message: &str) -> Js<T> {
    host::throw(g, name, message);
    Err(Thrown)
}

unsafe fn detached(g: *mut c_void, buffer: u64) -> bool {
    let mut info = RjsAbInfo::default();
    glue::rjs_arraybuffer_info(g, buffer, &mut info) != 0 || info.detached != 0
}

unsafe fn uint8_array(g: *mut c_void, buffer: u64) -> Js<u64> {
    construct(g, get(g, glue::rjs_global(g), "Uint8Array"), &[buffer])
}

/// A `Uint8Array` over `region`, without a copy.
unsafe fn bytes_out(g: *mut c_void, region: &Region, fresh: bool) -> Js<u64> {
    match memory::array_buffer(g, region, fresh) {
        Some(buffer) => uint8_array(g, buffer),
        None => range_error(
            g,
            &format!("{} bytes are more than an ArrayBuffer holds", region.len()),
        ),
    }
}

struct ImageState {
    width: u32,
    height: u32,
    /// `None` once closed.
    region: Option<Region>,
    /// Resizes and encodes reading the pixels on a worker; `pixels` is detached meanwhile.
    busy: u32,
}

host_state!(image_state, image_create, ImageState, IMAGE, "Image");

unsafe fn make_image(g: *mut c_void, pixels: Pixels) -> Js<u64> {
    let region = Region::from_vec(pixels.rgba);
    let Some(buffer) = memory::array_buffer(g, &region, true) else {
        return range_error(g, "the decoded pixels are more than an ArrayBuffer holds");
    };
    let view = uint8_array(g, buffer)?;
    let object = image_create(
        g,
        ImageState {
            width: pixels.width,
            height: pixels.height,
            region: Some(region),
            busy: 0,
        },
    );
    slot_set(g, object, BUFFER, buffer);
    slot_set(g, object, VIEW, view);
    Ok(object)
}

/// The image's state, closed if JS transferred the `ArrayBuffer` of its pixels away.
unsafe fn current<'a>(g: *mut c_void, this: u64) -> Js<&'a mut ImageState> {
    let state = image_state(g, this)?;
    if state.region.is_some() && state.busy == 0 && detached(g, slot_get(g, this, BUFFER)) {
        state.region = None;
    }
    Ok(state)
}

/// The pixels for a worker to read; `pixels` is detached until the last such reader gives them back.
unsafe fn lend(g: *mut c_void, this: u64, what: &str) -> Js<(Region, u32, u32)> {
    let state = current(g, this)?;
    let Some(region) = state.region.clone() else {
        return dom(
            g,
            "InvalidStateError",
            &format!("{what}: the image is closed"),
        );
    };
    if state.busy == 0 {
        glue::rjs_arraybuffer_detach(g, slot_get(g, this, BUFFER));
    }
    state.busy += 1;
    Ok((region, state.width, state.height))
}

unsafe fn give_back(g: *mut c_void, this: u64) {
    let Ok(state) = image_state(g, this) else {
        return;
    };
    state.busy -= 1;
    if state.busy != 0 {
        return;
    }
    let Some(region) = state.region.clone() else {
        return;
    };
    let fresh = memory::array_buffer(g, &region, false)
        .and_then(|buffer| Some((buffer, uint8_array(g, buffer).ok()?)));
    match fresh {
        Some((buffer, view)) => {
            slot_set(g, this, BUFFER, buffer);
            slot_set(g, this, VIEW, view);
        }
        None => {
            if let Ok(state) = image_state(g, this) {
                state.region = None;
            }
        }
    }
}

std_fn!(fn image_width(g, args) {
    let state = current(g, args.this().0)?;
    Ok(enc_f64(if state.region.is_some() { state.width as f64 } else { 0.0 }))
});

std_fn!(fn image_height(g, args) {
    let state = current(g, args.this().0)?;
    Ok(enc_f64(if state.region.is_some() { state.height as f64 } else { 0.0 }))
});

std_fn!(fn image_pixels(g, args) {
    let this = args.this().0;
    if current(g, this)?.busy != 0 {
        return dom(g, "InvalidStateError", "Image.pixels: a resize or an encode is reading the pixels; await it first");
    }
    Ok(slot_get(g, this, VIEW))
});

std_fn!(fn image_close(g, args) {
    let this = args.this().0;
    let state = current(g, this)?;
    if state.region.take().is_some() && state.busy == 0 {
        glue::rjs_arraybuffer_detach(g, slot_get(g, this, BUFFER));
    }
    Ok(UNDEF)
});

std_fn!(fn image_to_bitmap(g, args) {
    let this = args.this().0;
    let what = "Image.toImageBitmap";
    let state = current(g, this)?;
    if state.busy != 0 {
        return dom(g, "InvalidStateError", &format!("{what}: a resize or an encode is reading the pixels; await it first"));
    }
    let Some(region) = state.region.take() else {
        return dom(g, "InvalidStateError", &format!("{what}: the image is closed"));
    };
    let (width, height) = (state.width, state.height);
    // Detaching lets go of the ArrayBuffer's count, so the image is the chunk's only holder.
    glue::rjs_arraybuffer_detach(g, slot_get(g, this, BUFFER));
    let rgba = region.into_vec().unwrap_or_else(|region| region.bytes().to_vec());
    Ok(web_image::create_bitmap(g, web_image::BitmapData::new(width, height, rgba)))
});

/// A side of a resize: an integer from 1 to the image's own.
unsafe fn side(g: *mut c_void, v: u64, most: u32, what: &str) -> Js<u32> {
    let n = count(g, v, what)?;
    if n == 0 || n > most as usize {
        return range_error(
            g,
            &format!("{what} must be from 1 to {most}; an image is only made smaller"),
        );
    }
    Ok(n as u32)
}

std_fn!(fn image_resize(g, args) {
    let this = args.this().0;
    let (w, h) = (args.arg(0).0, args.arg(1).0);
    Ok(promise_of(g, || {
        let state = current(g, this)?;
        if state.region.is_none() {
            return dom(g, "InvalidStateError", "Image.resize: the image is closed");
        }
        let (from_w, from_h) = (state.width, state.height);
        let width = side(g, w, from_w, "Image.resize: the width")?;
        let height = side(g, h, from_h, "Image.resize: the height")?;
        let (region, from_w, from_h) = lend(g, this, "Image.resize")?;
        redjs_protect(g, this);
        let (promise, settler) = host::deferred(g);
        on_worker(
            g,
            move || {
                // SAFETY: `region` holds the bytes, and JS cannot reach them while they are lent.
                let bytes = unsafe { region.bytes() };
                Ok(sk_resample::downscale(bytes, from_w, from_h, width, height, false))
            },
            move |g, result| unsafe {
                give_back(g, this);
                redjs_unprotect(g, this);
                let made = result.map(|rgba| make_image(g, Pixels { width, height, rgba }));
                match made {
                    Ok(Ok(image)) => settler.resolve(g, image),
                    Ok(Err(Thrown)) => settler.reject(g, pending_exception(g)),
                    Err(f) => settler.reject(g, failure(g, &f)),
                }
            },
        );
        Ok(promise)
    }))
});

enum Data {
    Streaming(Vec<u8>),
    Ready(Arc<[u8]>),
    /// The stream failed with this value, kept protected.
    Failed(u64),
    Closed,
}

enum Want {
    Frame { index: usize, max_side: Option<u32> },
    Tracks,
}

struct DecoderState {
    mime: &'static str,
    format: Codec,
    data: Data,
    /// What was asked for before the stream ended.
    waiting: Vec<(Want, host::Settler)>,
}

host_state!(
    decoder_state,
    decoder_create,
    DecoderState,
    DECODER,
    "ImageDecoder"
);

fn closed(state: &DecoderState) -> bool {
    matches!(state.data, Data::Closed)
}

unsafe fn submit(
    g: *mut c_void,
    this: u64,
    data: Arc<[u8]>,
    format: Codec,
    want: Want,
    settler: host::Settler,
) {
    redjs_protect(g, this);
    match want {
        Want::Frame { index, max_side } => on_worker(
            g,
            move || decode(&data, format, index, max_side),
            move |g, result| unsafe {
                redjs_unprotect(g, this);
                if decoder_state(g, this).map_or(true, |s| closed(s)) {
                    return settler.reject(
                        g,
                        host::error(
                            g,
                            "AbortError",
                            "ImageDecoder.decode: the decoder was closed",
                        ),
                    );
                }
                match result {
                    Ok(pixels) => match make_image(g, pixels) {
                        Ok(image) => {
                            let out = new_object(g, UNDEF);
                            define(g, out, "image", image, 0);
                            define(g, out, "complete", TRUE, 0);
                            settler.resolve(g, out);
                        }
                        Err(Thrown) => settler.reject(g, pending_exception(g)),
                    },
                    Err(f) => settler.reject(g, failure(g, &f)),
                }
            },
        ),
        Want::Tracks => on_worker(
            g,
            move || probe(&data, format),
            move |g, result| unsafe {
                redjs_unprotect(g, this);
                if decoder_state(g, this).map_or(true, |s| closed(s)) {
                    return settler.reject(
                        g,
                        host::error(
                            g,
                            "AbortError",
                            "ImageDecoder.tracks: the decoder was closed",
                        ),
                    );
                }
                match result {
                    Ok((frames, animated)) => {
                        let tracks = slot_get(g, this, TRACKS);
                        define(g, tracks, "frameCount", enc_f64(frames as f64), 0);
                        define(g, tracks, "animated", enc_bool(animated), 0);
                        settler.resolve(g, UNDEF);
                    }
                    Err(f) => settler.reject(g, failure(g, &f)),
                }
            },
        ),
    }
}

/// Starts `want` now if the data is all there, later if the stream is still arriving.
unsafe fn ask(g: *mut c_void, this: u64, want: Want, settler: host::Settler) -> Js<()> {
    let state = decoder_state(g, this)?;
    match &state.data {
        Data::Ready(data) => submit(g, this, data.clone(), state.format, want, settler),
        Data::Streaming(_) => state.waiting.push((want, settler)),
        Data::Failed(error) => settler.reject(g, *error),
        Data::Closed => settler.reject(
            g,
            host::error(
                g,
                "InvalidStateError",
                "ImageDecoder: the decoder is closed",
            ),
        ),
    }
    Ok(())
}

unsafe fn stream_failed(g: *mut c_void, this: u64, error: u64) {
    let Ok(state) = decoder_state(g, this) else {
        return;
    };
    if !matches!(state.data, Data::Streaming(_)) {
        return;
    }
    redjs_protect(g, error);
    state.data = Data::Failed(error);
    for (_, settler) in std::mem::take(&mut state.waiting) {
        settler.reject(g, error);
    }
}

unsafe fn cancel_stream(g: *mut c_void, this: u64) {
    let reader = slot_get(g, this, READER);
    if is_cell(reader) {
        let _ = call(g, get(g, reader, "cancel"), reader, &[]);
        slot_set(g, this, READER, UNDEF);
    }
}

unsafe fn pull(g: *mut c_void, this: u64) -> Js<()> {
    let reader = slot_get(g, this, READER);
    let read = call(g, get(g, reader, "read"), reader, &[]).map_err(|_| Thrown)?;
    let then = get(g, read, "then");
    call(
        g,
        then,
        read,
        &[slot_get(g, this, ON_CHUNK), slot_get(g, this, ON_FAIL)],
    )
    .map_err(|_| Thrown)?;
    Ok(())
}

// `this` is the decoder, bound when the stream was taken.
std_fn!(fn stream_chunk(g, args) {
    let this = args.this().0;
    if !matches!(decoder_state(g, this)?.data, Data::Streaming(_)) {
        return Ok(UNDEF);
    }
    let result = args.arg(0).0;
    let done = to_bool(g, get(g, result, "done"));
    let value = get(g, result, "value");
    let outcome = (|| -> Js<bool> {
        if done {
            return Ok(true);
        }
        let bytes = bytes_arg(g, value, "ImageDecoder: a chunk of the stream")?;
        let state = decoder_state(g, this)?;
        let Data::Streaming(data) = &mut state.data else {
            return Ok(false);
        };
        if data.len() + bytes.len() > MAX_ENCODED {
            return range_error(g, &format!("ImageDecoder: the stream is longer than {MAX_ENCODED} bytes"));
        }
        data.extend_from_slice(bytes);
        Ok(false)
    })();
    match outcome {
        Ok(true) => {
            slot_set(g, this, READER, UNDEF);
            let state = decoder_state(g, this)?;
            let Data::Streaming(data) = std::mem::replace(&mut state.data, Data::Closed) else {
                return Ok(UNDEF);
            };
            let data: Arc<[u8]> = data.into();
            state.data = Data::Ready(data.clone());
            let format = state.format;
            for (want, settler) in std::mem::take(&mut state.waiting) {
                submit(g, this, data.clone(), format, want, settler);
            }
        }
        Ok(false) => {
            if pull(g, this).is_err() {
                stream_failed(g, this, pending_exception(g));
            }
        }
        Err(Thrown) => {
            let error = pending_exception(g);
            cancel_stream(g, this);
            stream_failed(g, this, error);
        }
    }
    Ok(UNDEF)
});

std_fn!(fn stream_fail(g, args) {
    stream_failed(g, args.this().0, args.arg(0).0);
    Ok(UNDEF)
});

unsafe fn bound(g: *mut c_void, name: &str, f: HostFn, this: u64) -> Js<u64> {
    let function = make_fn(g, name, 1, f);
    call(g, get(g, function, "bind"), function, &[this]).map_err(|_| Thrown)
}

std_fn!(fn decoder_new(g, args) {
    let what = "ImageDecoder";
    let init = args.arg(0).0;
    if !is_cell(init) {
        return type_error(g, "ImageDecoder: the argument must be an object with data and type");
    }
    let data = get(g, init, "data");
    let mime = string(g, get(g, init, "type"), "ImageDecoder: type")?;
    let Some(format) = codec_of(&mime) else {
        return dom(g, "NotSupportedError", &format!("ImageDecoder: {mime:?} is not a type this host decodes"));
    };
    let mime = canonical(format);
    let stream = is_cell(data) && kind(g, get(g, data, "getReader")) == KIND_FUNCTION;
    let bytes = if stream {
        Data::Streaming(Vec::new())
    } else {
        let bytes = bytes_arg(g, data, "ImageDecoder: data")?;
        if bytes.is_empty() {
            return type_error(g, "ImageDecoder: data is empty");
        }
        if bytes.len() > MAX_ENCODED {
            return range_error(g, &format!("{what}: data is longer than {MAX_ENCODED} bytes"));
        }
        Data::Ready(bytes.into())
    };
    let object = decoder_create(g, DecoderState { mime, format, data: bytes, waiting: Vec::new() });
    if stream {
        let reader = call(g, get(g, data, "getReader"), data, &[]).map_err(|_| Thrown)?;
        slot_set(g, object, READER, reader);
        slot_set(g, object, ON_CHUNK, bound(g, "chunk", stream_chunk, object)?);
        slot_set(g, object, ON_FAIL, bound(g, "fail", stream_fail, object)?);
        pull(g, object)?;
    }
    Ok(object)
});

std_fn!(fn decoder_type(g, args) {
    Ok(js_string(g, decoder_state(g, args.this().0)?.mime))
});

std_fn!(fn decoder_complete(g, args) {
    Ok(enc_bool(matches!(decoder_state(g, args.this().0)?.data, Data::Ready(_))))
});

std_fn!(fn decoder_tracks(g, args) {
    let this = args.this().0;
    decoder_state(g, this)?;
    let tracks = slot_get(g, this, TRACKS);
    if is_cell(tracks) {
        return Ok(tracks);
    }
    let tracks = new_object(g, UNDEF);
    define(g, tracks, "frameCount", enc_f64(0.0), 0);
    define(g, tracks, "animated", FALSE, 0);
    let (ready, settler) = host::deferred(g);
    define(g, tracks, "ready", ready, 0);
    slot_set(g, this, TRACKS, tracks);
    ask(g, this, Want::Tracks, settler)?;
    Ok(tracks)
});

std_fn!(fn decoder_decode(g, args) {
    let this = args.this().0;
    let options = args.arg(0).0;
    Ok(promise_of(g, || {
        decoder_state(g, this)?;
        let index = match option(g, options, "frameIndex") {
            UNDEF => 0,
            v => count(g, v, "ImageDecoder.decode: frameIndex")?,
        };
        let max_side = match option(g, options, "maxSide") {
            UNDEF => None,
            v => match count(g, v, "ImageDecoder.decode: maxSide")? {
                0 => return range_error(g, "ImageDecoder.decode: maxSide must be at least 1"),
                n => Some(n.min(MAX_SIDE as usize) as u32),
            },
        };
        let (promise, settler) = host::deferred(g);
        ask(g, this, Want::Frame { index, max_side }, settler)?;
        Ok(promise)
    }))
});

std_fn!(fn decoder_close(g, args) {
    let this = args.this().0;
    let state = decoder_state(g, this)?;
    let was = std::mem::replace(&mut state.data, Data::Closed);
    let waiting = std::mem::take(&mut state.waiting);
    match was {
        Data::Streaming(_) => cancel_stream(g, this),
        Data::Failed(error) => redjs_unprotect(g, error),
        _ => {}
    }
    for (_, settler) in waiting {
        settler.reject(g, host::error(g, "AbortError", "ImageDecoder: the decoder was closed"));
    }
    Ok(UNDEF)
});

std_fn!(fn decoder_is_type_supported(g, args) {
    let v = args.arg(0).0;
    Ok(promise_of(g, || {
        let mime = string(g, v, "ImageDecoder.isTypeSupported: the type")?;
        Ok(host::resolved(g, enc_bool(codec_of(&mime).is_some())))
    }))
});

fn encodable(mime: &str) -> Option<&'static str> {
    let mime = canonical(codec_of(mime)?);
    ENCODABLE.contains(&mime).then_some(mime)
}

std_fn!(fn encoder_is_type_supported(g, args) {
    let mime = string(g, args.arg(0).0, "ImageEncoder.isTypeSupported: the type")?;
    Ok(enc_bool(encodable(&mime).is_some()))
});

std_fn!(fn encoder_encode(g, args) {
    let source = args.arg(0).0;
    let options = args.arg(1).0;
    Ok(promise_of(g, || {
        let what = "ImageEncoder.encode";
        let mime = string(g, option(g, options, "type"), "ImageEncoder.encode: type")?;
        let Some(mime) = encodable(&mime) else {
            return dom(g, "NotSupportedError", &format!("{what}: {mime:?} is not a type this host encodes; it encodes {}", ENCODABLE.join(", ")));
        };
        let quality = match option(g, options, "quality") {
            UNDEF => None,
            v => {
                let q = number(g, v)?;
                if !(0.0..=1.0).contains(&q) {
                    return range_error(g, &format!("{what}: quality must be from 0 to 1"));
                }
                Some(q)
            }
        };
        let (promise, settler) = host::deferred(g);
        if host_data::<ImageState>(g, host_class::<ImageState>(&IMAGE, "Image"), source).is_some() {
            let (region, width, height) = lend(g, source, what)?;
            redjs_protect(g, source);
            on_worker(
                g,
                move || {
                    // SAFETY: `region` holds the bytes, and JS cannot reach them while they are lent.
                    encode(unsafe { region.bytes() }, width, height, mime, quality)
                },
                move |g, result| unsafe {
                    give_back(g, source);
                    redjs_unprotect(g, source);
                    settle_bytes(g, settler, result);
                },
            );
            return Ok(promise);
        }
        let Some((rgba, width, height)) = web_image::source_pixels(g, source) else {
            return type_error(g, &format!("{what}: the image must be an Image or an ImageBitmap"));
        };
        if width == 0 || height == 0 {
            return dom(g, "InvalidStateError", &format!("{what}: the image has no pixels (an ImageBitmap that was closed?)"));
        }
        on_worker(g, move || encode(&rgba, width, height, mime, quality), move |g, result| unsafe {
            settle_bytes(g, settler, result);
        });
        Ok(promise)
    }))
});

unsafe fn settle_bytes(g: *mut c_void, settler: host::Settler, result: Result<Vec<u8>, Failure>) {
    match result.map(|bytes| bytes_out(g, &Region::from_vec(bytes), true)) {
        Ok(Ok(array)) => settler.resolve(g, array),
        Ok(Err(Thrown)) => settler.reject(g, pending_exception(g)),
        Err(f) => settler.reject(g, failure(g, &f)),
    }
}

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let global = host::global(g);
    // `support::class` takes its name off the global object, where `Image` is the HTML one.
    let html_image = get(g, global, "Image");
    let image = support::class(
        g,
        &ClassSpec {
            name: "Image",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("resize", 2, image_resize),
                ("toImageBitmap", 0, image_to_bitmap),
                ("close", 0, image_close),
            ],
            getters: &[
                ("width", image_width),
                ("height", image_height),
                ("pixels", image_pixels),
            ],
            accessors: &[],
        },
    );
    if html_image != UNDEF {
        define(g, global, "Image", html_image, 2);
    }
    let decoder = support::class(
        g,
        &ClassSpec {
            name: "ImageDecoder",
            arity: 1,
            construct: Some(decoder_new),
            parent_proto: 0,
            methods: &[("decode", 0, decoder_decode), ("close", 0, decoder_close)],
            getters: &[
                ("type", decoder_type),
                ("complete", decoder_complete),
                ("tracks", decoder_tracks),
            ],
            accessors: &[],
        },
    );
    static_fn(
        g,
        decoder.ctor,
        "isTypeSupported",
        1,
        decoder_is_type_supported,
    );
    let encoder = support::class(
        g,
        &ClassSpec {
            name: "ImageEncoder",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[],
            getters: &[],
            accessors: &[],
        },
    );
    static_fn(g, encoder.ctor, "encode", 2, encoder_encode);
    static_fn(
        g,
        encoder.ctor,
        "isTypeSupported",
        1,
        encoder_is_type_supported,
    );

    let module = new_object(g, UNDEF);
    define(g, module, "Image", image.ctor, 0);
    define(g, module, "ImageDecoder", decoder.ctor, 0);
    define(g, module, "ImageEncoder", encoder.ctor, 0);
    define(g, std, "image", module, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::time::{Duration, Instant};

    const TRANSLUCENT: [u8; 4] = [10, 20, 30, 128];

    fn file(img: &RgbaImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn two_pixels() -> RgbaImage {
        RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba(TRANSLUCENT)
            }
        })
    }

    fn gif(frames: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for colour in frames {
                let frame = image::Frame::new(RgbaImage::from_pixel(4, 2, Rgba(*colour)));
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    fn near(a: &[u8], b: &[u8], by: u8) -> bool {
        a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= by)
    }

    #[test]
    fn each_format_decodes_to_its_pixels() {
        let img = two_pixels();
        for format in [ImageFormat::Png, ImageFormat::WebP] {
            let p = decode(&file(&img, format), Raster(format), 0, None).unwrap();
            assert_eq!((p.width, p.height), (2, 1), "{format:?}");
            assert_eq!(
                p.rgba,
                img.as_raw().as_slice(),
                "{format:?}: alpha must stay straight"
            );
        }
        let opaque = RgbaImage::from_fn(32, 16, |x, _| {
            if x < 16 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let rgb = image::DynamicImage::ImageRgba8(opaque.clone()).into_rgb8();
        let mut bmp = Cursor::new(Vec::new());
        rgb.write_to(&mut bmp, ImageFormat::Bmp).unwrap();
        let p = decode(bmp.get_ref(), Raster(ImageFormat::Bmp), 0, None).unwrap();
        assert_eq!(p.rgba, opaque.as_raw().as_slice());
        let mut jpeg = Cursor::new(Vec::new());
        rgb.write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
        let p = decode(jpeg.get_ref(), Raster(ImageFormat::Jpeg), 0, None).unwrap();
        assert_eq!((p.width, p.height), (32, 16));
        assert!(
            near(&p.rgba[..4], &[255, 0, 0, 255], 24),
            "{:?}",
            &p.rgba[..4]
        );
        assert!(near(&p.rgba[31 * 4..32 * 4], &[0, 0, 255, 255], 24));
    }

    #[test]
    fn a_photo_is_turned_upright_by_its_exif_orientation() {
        let rgb = image::RgbImage::from_fn(32, 16, |x, _| {
            if x < 16 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut out = Cursor::new(Vec::new());
        rgb.write_to(&mut out, ImageFormat::Jpeg).unwrap();
        let jpeg = out.into_inner();
        // Orientation 6 (turn 90 degrees clockwise) in a big-endian TIFF of one IFD entry.
        let tiff = [
            0x4D, 0x4D, 0, 0x2A, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0,
            0, 0,
        ];
        let len = (2 + 6 + tiff.len()) as u16;
        let mut turned = jpeg[..2].to_vec();
        turned.extend_from_slice(&[0xFF, 0xE1, (len >> 8) as u8, len as u8]);
        turned.extend_from_slice(b"Exif\0\0");
        turned.extend_from_slice(&tiff);
        turned.extend_from_slice(&jpeg[2..]);
        let p = decode(&turned, Raster(ImageFormat::Jpeg), 0, None).unwrap();
        assert_eq!((p.width, p.height), (16, 32));
        let at = |x: usize, y: usize| &p.rgba[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert!(
            at(8, 4)[0] > 200 && at(8, 4)[2] < 60,
            "red on top: {:?}",
            at(8, 4)
        );
        assert!(
            at(8, 27)[2] > 200 && at(8, 27)[0] < 60,
            "blue below: {:?}",
            at(8, 27)
        );
    }

    #[test]
    fn an_animation_has_its_frames_counted_and_decoded_one_by_one() {
        let bytes = gif(&[[255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255]]);
        assert_eq!(probe(&bytes, Raster(ImageFormat::Gif)).unwrap(), (3, true));
        let second = decode(&bytes, Raster(ImageFormat::Gif), 1, None).unwrap();
        assert_eq!((second.width, second.height), (4, 2));
        assert!(
            near(&second.rgba[..4], &[0, 0, 255, 255], 8),
            "{:?}",
            &second.rgba[..4]
        );
        assert!(matches!(
            decode(&bytes, Raster(ImageFormat::Gif), 3, None),
            Err(Failure::Range(_))
        ));
        let still = file(&two_pixels(), ImageFormat::Png);
        assert_eq!(probe(&still, Raster(ImageFormat::Png)).unwrap(), (1, false));
        assert!(matches!(
            decode(&still, Raster(ImageFormat::Png), 1, None),
            Err(Failure::Range(_))
        ));
        assert_eq!(
            probe(&gif(&[[1, 2, 3, 255]]), Raster(ImageFormat::Gif)).unwrap(),
            (1, false)
        );
    }

    /// A PNG whose header says `width` x `height`, with a valid CRC and no pixel data.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&width.to_be_bytes());
        chunk.extend_from_slice(&height.to_be_bytes());
        chunk.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut crc = flate2::Crc::new();
        crc.update(&chunk);
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc.sum().to_be_bytes());
        out
    }

    /// An ICO of one entry holding `image`.
    fn ico(image: &[u8]) -> Vec<u8> {
        let mut out = vec![0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 32, 0];
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&22u32.to_le_bytes());
        out.extend_from_slice(image);
        out
    }

    /// A DXT1 DDS of `width` x `height` followed by `blocks`.
    fn dds(width: u32, height: u32, blocks: &[u8]) -> Vec<u8> {
        let mut out = b"DDS ".to_vec();
        for word in [124, 0x1007, height, width, 0, 0, 0] {
            out.extend_from_slice(&u32::to_le_bytes(word));
        }
        out.extend_from_slice(&[0; 44]);
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(b"DXT1");
        out.extend_from_slice(&[0; 20]);
        out.extend_from_slice(&0x1000u32.to_le_bytes());
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(blocks);
        out
    }

    /// A little-endian RGB TIFF whose one IFD says `width` x `height`, with no pixel data.
    fn tiff_header(width: u32, height: u32) -> Vec<u8> {
        let entries: [(u16, u16, u32); 9] = [
            (256, 4, width),
            (257, 4, height),
            (258, 3, 8),
            (259, 3, 1),
            (262, 3, 2),
            (273, 4, 8),
            (277, 3, 3),
            (278, 4, height),
            (279, 4, 0),
        ];
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, value) in entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&[0; 4]);
        out
    }

    #[test]
    fn the_other_types_round_trip_or_decode_to_their_pixels() {
        let (red, blue, clear) = ([255, 0, 0, 255], [0, 0, 255, 255], [0, 0, 0, 0]);
        let img = RgbaImage::from_fn(2, 2, |x, y| {
            Rgba([red, blue, clear, TRANSLUCENT][(y * 2 + x) as usize])
        });
        let opaque = |p: [u8; 4]| [p[0], p[1], p[2], 255];
        let binary = |p: [u8; 4]| if p[3] == 0 { p } else { opaque(p) };
        let same = |p: [u8; 4]| p;
        let round_trips: [(&str, &dyn Fn([u8; 4]) -> [u8; 4], u8); 9] = [
            ("image/gif", &binary, 0),
            ("image/x-icon", &same, 0),
            ("image/bmp", &same, 0),
            ("image/qoi", &same, 0),
            ("image/tiff", &same, 0),
            ("image/x-tga", &same, 0),
            ("image/x-portable-anymap", &same, 0),
            ("image/x-farbfeld", &same, 0),
            ("image/vnd.radiance", &opaque, 1),
        ];
        for (mime, expect, by) in round_trips {
            let encoded = encode(img.as_raw(), 2, 2, encodable(mime).unwrap(), Some(0.1)).unwrap();
            let p = decode(&encoded, codec_of(mime).unwrap(), 0, None).unwrap();
            let want: Vec<u8> = img.pixels().flat_map(|p| expect(p.0)).collect();
            assert_eq!((p.width, p.height), (2, 2), "{mime}");
            assert!(near(&p.rgba, &want, by), "{mime}: {:?}", p.rgba);
        }
        assert!(matches!(
            encode(&vec![0; 257 * 4], 257, 1, "image/x-icon", None),
            Err(Failure::Encoding(_))
        ));

        let mut cur = encode(img.as_raw(), 2, 2, "image/x-icon", None).unwrap();
        cur[2] = 2;
        cur[10..14].copy_from_slice(&[1, 0, 1, 0]);
        let p = decode(&cur, codec_of("image/x-icon").unwrap(), 0, None).unwrap();
        assert_eq!(p.rgba, img.as_raw().as_slice(), "a cursor reads as an icon");

        let only_decoded: [(&str, Vec<u8>, &[u8]); 3] = [
            ("image/x-portable-pixmap", [&b"P6\n2 1\n255\n"[..], &[255, 0, 0, 0, 0, 255]].concat(), &[255, 0, 0, 255, 0, 0, 255, 255]),
            // Colour 0 red, colour 1 blue; the first row reads 0, 1, 0, 1.
            ("image/vnd.ms-dds", dds(4, 4, &[0x00, 0xF8, 0x1F, 0x00, 0x44, 0, 0, 0]), &[255, 0, 0, 255, 0, 0, 255, 255]),
            (
                "image/svg+xml",
                br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="1" height="1" fill="#f00"/><rect x="1" width="1" height="1" fill="#00f"/></svg>"##.to_vec(),
                &[255, 0, 0, 255, 0, 0, 255, 255],
            ),
        ];
        for (mime, bytes, first) in only_decoded {
            let codec = codec_of(mime).unwrap();
            let p = decode(&bytes, codec, 0, None).unwrap();
            assert_eq!(&p.rgba[..8], first, "{mime}");
            assert_eq!(probe(&bytes, codec).unwrap(), (1, false), "{mime}");
        }

        let wide = br##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200"><rect width="200" height="200" fill="#f00"/><rect x="200" width="200" height="200" fill="#00f"/></svg>"##;
        let p = decode(wide, Svg, 0, Some(10)).unwrap();
        assert_eq!((p.width, p.height, p.rgba.len()), (10, 5, 200));
        assert_eq!((&p.rgba[..4], &p.rgba[36..40]), (&red[..], &blue[..]));
    }

    #[test]
    fn hostile_files_fail_without_allocating_their_claimed_size() {
        for (w, h) in [
            (MAX_SIDE + 1, 1),
            (12000, 12000),
            (0x7fff_ffff, 0x7fff_ffff),
        ] {
            let (w16, h16) = (w.min(0xffff) as u16, h.min(0xffff) as u16);
            let headers = [
                ("image/png", png_header(w, h)),
                ("image/x-icon", ico(&png_header(w, h))),
                (
                    "image/qoi",
                    [&b"qoif"[..], &w.to_be_bytes(), &h.to_be_bytes(), &[4, 0]].concat(),
                ),
                (
                    "image/x-farbfeld",
                    [&b"farbfeld"[..], &w.to_be_bytes(), &h.to_be_bytes()].concat(),
                ),
                (
                    "image/x-tga",
                    [
                        &[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0][..],
                        &w16.to_le_bytes(),
                        &h16.to_le_bytes(),
                        &[32, 8],
                    ]
                    .concat(),
                ),
                (
                    "image/x-portable-anymap",
                    format!("P6\n{w} {h}\n255\n").into_bytes(),
                ),
                (
                    "image/vnd.radiance",
                    format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {h} +X {w}\n").into_bytes(),
                ),
                (
                    "image/vnd-ms.dds",
                    dds(w.next_multiple_of(4), h.next_multiple_of(4), &[]),
                ),
                ("image/tiff", tiff_header(w, h)),
                (
                    "image/svg+xml",
                    format!(
                        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"/>"#
                    )
                    .into_bytes(),
                ),
            ];
            for (mime, header) in headers {
                let codec = codec_of(mime).unwrap();
                assert!(
                    matches!(decode(&header, codec, 0, None), Err(Failure::Encoding(_))),
                    "{mime} {w}x{h}"
                );
                assert!(
                    matches!(probe(&header, codec), Err(Failure::Encoding(_))),
                    "{mime} {w}x{h}"
                );
            }
        }
        assert!(matches!(
            decode(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="4"/>"#,
                Svg,
                0,
                None
            ),
            Err(Failure::Encoding(_))
        ));
        let whole = file(
            &RgbaImage::from_pixel(64, 64, Rgba([9, 9, 9, 255])),
            ImageFormat::Png,
        );
        for cut in [8, 40, whole.len() / 2, whole.len() - 13] {
            assert!(
                matches!(
                    decode(&whole[..cut], Raster(ImageFormat::Png), 0, None),
                    Err(Failure::Encoding(_))
                ),
                "cut at {cut}"
            );
        }
        let animation = gif(&[[255, 0, 0, 255], [0, 0, 255, 255]]);
        assert!(decode(
            &animation[..animation.len() - 20],
            Raster(ImageFormat::Gif),
            1,
            None
        )
        .is_err());
        assert!(matches!(
            decode(b"not an image at all", Raster(ImageFormat::WebP), 0, None),
            Err(Failure::Encoding(_))
        ));
        assert!(
            matches!(
                decode(&whole, Raster(ImageFormat::Jpeg), 0, None),
                Err(Failure::Encoding(_))
            ),
            "the declared type decides"
        );
    }

    #[test]
    fn max_side_and_resizes_keep_the_shape_and_the_colour() {
        let big = RgbaImage::from_pixel(40, 20, Rgba(TRANSLUCENT));
        let p = decode(
            &file(&big, ImageFormat::Png),
            Raster(ImageFormat::Png),
            0,
            Some(10),
        )
        .unwrap();
        assert_eq!((p.width, p.height, p.rgba.len()), (10, 5, 200));
        assert!(near(&p.rgba[..4], &TRANSLUCENT, 1), "{:?}", &p.rgba[..4]);
        let small = decode(
            &file(&big, ImageFormat::Png),
            Raster(ImageFormat::Png),
            0,
            Some(100),
        )
        .unwrap();
        assert_eq!((small.width, small.height), (40, 20), "never enlarged");
        let tall = fit(
            Pixels {
                width: 3,
                height: 3000,
                rgba: vec![255; 3 * 3000 * 4],
            },
            100,
        );
        assert_eq!((tall.width, tall.height), (1, 100));
    }

    #[test]
    fn encoding_round_trips_and_refuses_what_it_cannot_write() {
        let img = two_pixels();
        let png = encode(img.as_raw(), 2, 1, "image/png", None).unwrap();
        assert_eq!(
            decode(&png, Raster(ImageFormat::Png), 0, None)
                .unwrap()
                .rgba,
            img.as_raw().as_slice()
        );
        let webp = encode(img.as_raw(), 2, 1, "image/webp", Some(0.1)).unwrap();
        assert_eq!(
            decode(&webp, Raster(ImageFormat::WebP), 0, None)
                .unwrap()
                .rgba,
            img.as_raw().as_slice(),
            "lossless"
        );
        let jpeg = encode(img.as_raw(), 2, 1, "image/jpeg", Some(0.9)).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert!(encodable("image/svg+xml").is_none() && encodable("image/vnd.ms-dds").is_none());
        assert_eq!(encodable(" IMAGE/PNG "), Some("image/png"));
        assert_eq!(encodable("image/x-targa"), Some("image/x-tga"));
    }

    #[test]
    fn region_into_vec_takes_the_memory_back_only_when_it_is_alone() {
        let v = vec![1u8, 2, 3];
        let at = v.as_ptr();
        let back = Region::from_vec(v).into_vec().ok().unwrap();
        assert_eq!((back.as_ptr(), back.as_slice()), (at, &[1u8, 2, 3][..]));
        let shared = Region::from_vec(vec![4]);
        let other = shared.clone();
        let refused = shared.into_vec().err().unwrap();
        drop(other);
        assert_eq!(refused.into_vec().ok().unwrap(), [4]);
    }

    #[test]
    fn decoding_runs_off_the_js_thread_and_settles_in_the_pump() {
        let png = file(&two_pixels(), ImageFormat::Png);
        sk_runtime::testing::on_js_thread(move || {
            sk_runtime::JsRuntime::with(|rt| {
                super::super::install(rt);
                let context = rt.ctx();
                let eval = |src: &str| -> String {
                    let v = context.eval(src).unwrap();
                    context.read_string(v).unwrap()
                };
                let bytes = png.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
                eval(&format!(
                    "(() => {{
                     const {{ ImageDecoder, ImageEncoder }} = __sk_std.image;
                     const {{ NativeBuffer }} = __sk_std.memory;
                     globalThis.__out = {{}};
                     const file = NativeBuffer.alloc({len});
                     new Uint8Array(file.buffer).set([{bytes}]);
                     const decoder = new ImageDecoder({{ data: file, type: 'image/png' }});
                     file.free();
                     globalThis.__decoder = decoder;
                     globalThis.__pending = decoder.decode().then(async ({{ image, complete }}) => {{
                       __out.decoded = [image.width, image.height, ...image.pixels, complete];
                       const resizing = image.resize(1, 1);
                       __out.during = (() => {{ try {{ image.pixels; return 'ok' }} catch (e) {{ return e.name }} }})();
                       const small = await resizing;
                       __out.small = [small.width, small.height, image.pixels.length];
                       const png = await ImageEncoder.encode(image, {{ type: 'image/png' }});
                       __out.encoded = [png instanceof Uint8Array, png[1], png[2], png[3]];
                       const bitmap = image.toImageBitmap();
                       __out.bitmap = [bitmap.width, bitmap.height, image.width, image.pixels.length];
                       __out.tooBig = await image.resize(1, 1).then(() => 'ok', (e) => e.name);
                     }}, (e) => {{ __out.failed = String(e); }});
                     return 'started';
                     }})()",
                    len = png.len()
                ));
                assert_eq!(
                    eval("(() => { const b = __sk_std.memory.NativeBuffer.alloc(4); b.free(); try { new __sk_std.image.ImageDecoder({ data: b, type: 'image/png' }); return 'ok'; } catch (e) { return e.name; } })()"),
                    "InvalidStateError"
                );
                assert_eq!(
                    eval("(() => { try { new __sk_std.image.ImageDecoder({ data: new Uint8Array(1), type: 'image/avif' }); return 'ok'; } catch (e) { return e.name; } })()"),
                    "NotSupportedError"
                );
                assert_eq!(eval("typeof Image === 'function' && Image !== __sk_std.image.Image ? 'html' : 'lost'"), "html");
                // Nothing runs on this thread while it waits, so the decode finishing proves a worker did it.
                let deadline = Instant::now() + Duration::from_secs(10);
                while FINISHED.lock().unwrap().is_empty() {
                    assert!(
                        Instant::now() < deadline,
                        "the decode did not finish on a worker"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(eval("JSON.stringify(__out)"), "{}");
                let deadline = Instant::now() + Duration::from_secs(10);
                while eval("String('bitmap' in __out || 'failed' in __out)") != "true"
                    || eval("String('tooBig' in __out || 'failed' in __out)") != "true"
                {
                    assert!(
                        Instant::now() < deadline,
                        "the promises did not settle: {}",
                        eval("JSON.stringify(__out)")
                    );
                    pump();
                    context.drain_microtasks();
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(
                    eval("JSON.stringify(__out)"),
                    r#"{"decoded":[2,1,255,0,0,255,10,20,30,128,true],"during":"InvalidStateError","small":[1,1,8],"encoded":[true,80,78,71],"bitmap":[2,1,0,0],"tooBig":"InvalidStateError"}"#
                );
            })
        });
    }
}
