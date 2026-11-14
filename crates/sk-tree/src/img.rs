//! `<img src>` — the browser's URL grammar, resolved against a native base.
//!
//! Relative `src`s resolve against the app root ([`set_asset_base`]) through [`content_key`], as
//! `fetch()` does; a packaged app looks them up in the payload store, never the disk. Every
//! URL-taking feature resolves through here.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use gpui::{Image, ImageFormat, ImageSource, Resource};
pub use sk_payload::content_key;
use sk_payload::KeyError;
use sk_runtime::redjs;
use sk_runtime::redjs::webapi::url::BlobTarget;
use sk_runtime::JsRuntime;

/// A packaged app's asset store: app-root-relative key → bytes (mapped in the exe for the process's life).
pub type AssetLookup = fn(&str) -> Option<&'static [u8]>;

/// Explicit imported files outside a dev app's root.
fn dev_asset_files() -> &'static RwLock<HashMap<String, PathBuf>> {
    static FILES: OnceLock<RwLock<HashMap<String, PathBuf>>> = OnceLock::new();
    FILES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Replaces the dev manifest's imported-file map, also visible to fetch's background threads. Only
/// canonical generated keys and absolute paths pass.
pub fn set_dev_asset_files(files: HashMap<String, PathBuf>) {
    let files = files
        .into_iter()
        .filter(|(key, path)| {
            key.starts_with("_vn/fs/")
                && path.is_absolute()
                && !key.contains(['\\', '\0'])
                && key
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
        })
        .collect();
    *dev_asset_files().write().unwrap() = files;
}

/// Resolves one explicitly mapped dev URL with the same normalization as image/font/video URLs and
/// fetch. Unknown keys never escape the app root.
pub fn dev_asset_file(src: &str) -> Option<PathBuf> {
    let key = content_key(src)?;
    dev_asset_files().read().unwrap().get(&key).cloned()
}

thread_local! {
    static BASE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static STORE: Cell<Option<AssetLookup>> = const { Cell::new(None) };
    static RESOLVED: RefCell<HashMap<u32, Memo>> = RefCell::new(HashMap::new());
    static BLOB_PATHS: RefCell<HashMap<PathBuf, Snapshot>> = RefCell::new(HashMap::new());
    /// Sources gpui's asset cache must drop at the next point with a `Window` ([`evict_pending`]).
    static EVICT: RefCell<Vec<ImageSource>> = const { RefCell::new(Vec::new()) };
}

type Snapshot = (u64, Option<std::time::SystemTime>);

/// What this module holds for devtools `mem`: (per-node memos, bytes of their src strings, distinct
/// in-memory encoded images, their bytes, CSS background entries). The decodes are gpui's.
pub(crate) fn mem_stats() -> (usize, usize, usize, usize, usize) {
    let mut seen = std::collections::HashSet::new();
    let (mut memos, mut src_bytes, mut images, mut image_bytes) = (0, 0, 0, 0);
    let mut count = |source: &ImageSource| {
        if let ImageSource::Image(img) = source {
            if seen.insert(img.id) {
                images += 1;
                image_bytes += img.bytes.capacity();
            }
        }
    };
    RESOLVED.with(|m| {
        for memo in m.borrow().values() {
            memos += 1;
            src_bytes += memo.src.capacity();
            if let Some(s) = &memo.source {
                count(s);
            }
        }
    });
    let backgrounds = BACKGROUNDS.with(|b| {
        let b = b.borrow();
        for layers in b.values() {
            for (_, s) in layers {
                if let Some(s) = s {
                    count(s);
                }
            }
        }
        b.values().map(Vec::len).sum::<usize>()
    });
    (memos, src_bytes, images, image_bytes, backgrounds)
}

struct Memo {
    src: String,
    source: Option<ImageSource>,
    blob: bool,
}

/// gpui caches a `Resource::Path` decode under the path and an `Image` under its id and drops neither:
/// a blob source is handed back when no node shows it, and a picked file whose snapshot changed
/// loses its stale decode.
fn release(map: &HashMap<u32, Memo>, gone: Memo) {
    let Memo {
        source: Some(source),
        blob: true,
        ..
    } = gone
    else {
        return;
    };
    if map
        .values()
        .any(|m| m.source.as_ref().is_some_and(|s| same_source(s, &source)))
    {
        return;
    }
    if let ImageSource::Resource(Resource::Path(p)) = &source {
        BLOB_PATHS.with(|b| b.borrow_mut().remove(p.as_ref()));
    }
    EVICT.with(|e| e.borrow_mut().push(source));
}

fn same_source(a: &ImageSource, b: &ImageSource) -> bool {
    match (a, b) {
        (ImageSource::Resource(x), ImageSource::Resource(y)) => x == y,
        (ImageSource::Image(x), ImageSource::Image(y)) => std::sync::Arc::ptr_eq(x, y),
        _ => false,
    }
}

/// Records that `path` goes to gpui as the file with `snap`; a decode of a different file is queued
/// for eviction.
fn note_blob_path(path: &Path, snap: Snapshot) {
    let stale = BLOB_PATHS
        .with(|b| b.borrow_mut().insert(path.to_path_buf(), snap))
        .is_some_and(|old| old != snap);
    if stale {
        EVICT.with(|e| e.borrow_mut().push(path_source(path.to_path_buf())));
    }
}

/// Hands queued sources back to gpui (the decode, and for a path its atlas tiles in every window).
/// Needs a `Window`: called after `<img>` resolves its `src`, and once per frame ([`crate::frame_tick`]).
pub(crate) fn evict_pending(window: &mut gpui::Window, cx: &mut gpui::App) {
    let queued = EVICT.with(|e| std::mem::take(&mut *e.borrow_mut()));
    for source in queued {
        if let ImageSource::Resource(r) = &source {
            if let Some(Ok(image)) = window.get_asset::<gpui::ImgResourceLoader>(r, cx) {
                cx.drop_image(image, Some(window));
            }
        }
        source.remove_asset(cx);
    }
}

// gpui keeps every decode until told otherwise; the sweep evicts unshown sources, reduces decodes
// drawn well below their size, and restores a reduced one that is drawn larger again.
const EVICT_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone, PartialEq, Eq, Hash)]
enum SourceKey {
    Resource(Resource),
    Image(u64),
}

fn source_key(source: &ImageSource) -> Option<SourceKey> {
    match source {
        ImageSource::Resource(r) => Some(SourceKey::Resource(r.clone())),
        ImageSource::Image(img) => Some(SourceKey::Image(img.id)),
        _ => None,
    }
}

#[derive(Default)]
struct Residency {
    known: HashMap<SourceKey, (ImageSource, std::time::Instant)>,
    restoring: std::collections::HashSet<SourceKey>,
    keep_full: std::collections::HashSet<SourceKey>,
    /// Reduced decodes replaced by their full decode, whose tiles go next tick.
    replaced: Vec<std::sync::Arc<gpui::RenderImage>>,
    restored: bool,
    last_sweep: Option<std::time::Instant>,
    stats: ResidencyStats,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct ResidencyStats {
    pub reduced: usize,
    pub reduced_freed: usize,
    pub restored: usize,
    pub evicted: usize,
    pub evicted_freed: usize,
    pub known: usize,
    pub kept_full: KeptFull,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct KeptFull {
    pub not_still: usize,
    pub undrawn: usize,
    pub young: usize,
    pub drawn_large: usize,
    pub no_copy: usize,
    pub small_saving: usize,
    pub canvas_read: usize,
    /// A window that keeps full decodes (`windows::WindowState::full_res_images`).
    pub full_res_window: usize,
}

impl KeptFull {
    fn count(&mut self, why: crate::resample::KeepFull) {
        use crate::resample::KeepFull as K;
        *match why {
            K::NotStill => &mut self.not_still,
            K::Undrawn => &mut self.undrawn,
            K::Young => &mut self.young,
            K::DrawnLarge => &mut self.drawn_large,
            K::NoCopy => &mut self.no_copy,
            K::SmallSaving => &mut self.small_saving,
        } += 1;
    }
}

thread_local! {
    static RESIDENCY: RefCell<Residency> = RefCell::new(Residency::default());
}

fn note_known(source: Option<&ImageSource>) {
    let Some(source) = source else { return };
    let Some(key) = source_key(source) else {
        return;
    };
    let now = std::time::Instant::now();
    RESIDENCY.with(|r| r.borrow_mut().known.insert(key, (source.clone(), now)));
}

pub fn residency_stats() -> ResidencyStats {
    RESIDENCY.with(|r| {
        let r = r.borrow();
        ResidencyStats {
            known: r.known.len(),
            ..r.stats
        }
    })
}

fn pixel_bytes(image: &gpui::RenderImage) -> usize {
    (0..image.frame_count())
        .map(|i| {
            let s = image.size(i);
            s.width.0.max(0) as usize * s.height.0.max(0) as usize * 4
        })
        .sum()
}

fn shown_sources() -> Vec<ImageSource> {
    let mut out: Vec<ImageSource> = RESOLVED.with(|m| {
        m.borrow()
            .values()
            .filter_map(|memo| memo.source.clone())
            .collect()
    });
    BACKGROUNDS.with(|b| {
        for layers in b.borrow().values() {
            out.extend(layers.iter().filter_map(|(_, s)| s.clone()));
        }
    });
    out
}

pub(crate) fn residency_tick(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    let now = std::time::Instant::now();
    let wanted = crate::resample::take_wanted_full();
    let (replaced, restored, due) = RESIDENCY.with(|r| {
        let mut r = r.borrow_mut();
        let due = r
            .last_sweep
            .is_none_or(|t| now.duration_since(t) >= SWEEP_EVERY);
        if due {
            r.last_sweep = Some(now);
        }
        (
            std::mem::take(&mut r.replaced),
            std::mem::replace(&mut r.restored, false),
            due,
        )
    });
    let mut moved_tiles = !replaced.is_empty();
    for image in replaced {
        cx.drop_image(image, Some(window));
    }
    if !wanted.is_empty() {
        restore(&wanted, cx);
    }
    if due {
        moved_tiles |= sweep(now, window, cx);
    }
    if moved_tiles {
        crate::resample::hold_replays_too();
    }
    restored
}

fn restore(wanted: &std::collections::HashSet<gpui::ImageId>, cx: &mut gpui::App) {
    let shown = shown_sources();
    for source in shown {
        let Some(key) = source_key(&source) else {
            continue;
        };
        let Some(cached) = source.cached(cx) else {
            continue;
        };
        if !cached.is_reduced() || !wanted.contains(&cached.id) {
            continue;
        }
        if !RESIDENCY.with(|r| r.borrow_mut().restoring.insert(key.clone())) {
            continue;
        }
        let Some(task) = source.load_detached(cx) else {
            RESIDENCY.with(|r| r.borrow_mut().restoring.remove(&key));
            continue;
        };
        cx.spawn(async move |cx| {
            let loaded = task.await;
            cx.update(|cx| {
                RESIDENCY.with(|r| r.borrow_mut().restoring.remove(&key));
                let Ok(full) = loaded else { return };
                let Some(reduced) = source.cached(cx).filter(|c| c.is_reduced()) else {
                    return;
                };
                if source.replace_cached(full, cx) {
                    RESIDENCY.with(|r| {
                        let mut r = r.borrow_mut();
                        r.replaced.push(reduced);
                        r.restored = true;
                        r.stats.restored += 1;
                    });
                }
            });
        })
        .detach();
    }
}

fn sweep(now: std::time::Instant, window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    let shown = shown_sources();
    crate::resample::prune_uses();
    let mut moved = false;
    RESIDENCY.with(|r| {
        let mut r = r.borrow_mut();
        for source in &shown {
            if let Some(key) = source_key(source) {
                r.known.insert(key, (source.clone(), now));
            }
        }
    });
    let stale: Vec<(SourceKey, ImageSource)> = RESIDENCY.with(|r| {
        let mut r = r.borrow_mut();
        let keys: Vec<SourceKey> = r
            .known
            .iter()
            .filter(|(_, (_, seen))| now.duration_since(*seen) >= EVICT_AFTER)
            .map(|(k, _)| k.clone())
            .collect();
        keys.into_iter()
            .filter_map(|k| r.known.remove(&k).map(|(s, _)| (k, s)))
            .collect()
    });
    for (key, source) in stale {
        if let Some(image) = source.cached(cx) {
            let bytes = pixel_bytes(&image);
            RESIDENCY.with(|r| {
                let mut r = r.borrow_mut();
                r.stats.evicted += 1;
                r.stats.evicted_freed += bytes;
            });
            cx.drop_image(image, Some(window));
            moved = true;
        }
        source.remove_asset(cx);
        RESIDENCY.with(|r| r.borrow_mut().keep_full.remove(&key));
    }
    let mut kept = KeptFull::default();
    let mut seen = std::collections::HashSet::new();
    let pinned = full_res_sources();
    for source in &shown {
        let Some(key) = source_key(source) else {
            continue;
        };
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(full) = source.cached(cx) else {
            continue;
        };
        if full.is_reduced() {
            continue;
        }
        if pinned.contains(&key) {
            kept.full_res_window += 1;
            continue;
        }
        let (canvas_read, restoring) = RESIDENCY.with(|r| {
            let r = r.borrow();
            (r.keep_full.contains(&key), r.restoring.contains(&key))
        });
        if canvas_read || restoring {
            kept.canvas_read += usize::from(canvas_read);
            continue;
        }
        let copy = match crate::resample::reduced_copy(&full, now) {
            Ok(copy) => copy,
            Err(why) => {
                kept.count(why);
                continue;
            }
        };
        let freed = pixel_bytes(&full).saturating_sub(pixel_bytes(&copy));
        if source.replace_cached(copy, cx) {
            RESIDENCY.with(|r| {
                let mut r = r.borrow_mut();
                r.stats.reduced += 1;
                r.stats.reduced_freed += freed;
            });
            cx.drop_image(full, Some(window));
            moved = true;
        }
    }
    RESIDENCY.with(|r| r.borrow_mut().stats.kept_full = kept);
    moved
}

/// Sources an `<img>` shows in a window that keeps full decodes: a media viewer zooms to 1:1, so a
/// reduced copy would blur until the restore landed.
fn full_res_sources() -> std::collections::HashSet<SourceKey> {
    if !crate::windows::any_full_res() {
        return std::collections::HashSet::new();
    }
    let nodes: Vec<(u32, SourceKey)> = RESOLVED.with(|m| {
        m.borrow()
            .iter()
            .filter_map(|(id, memo)| Some((*id, source_key(memo.source.as_ref()?)?)))
            .collect()
    });
    nodes
        .into_iter()
        .filter(|(id, _)| {
            let k = crate::windows::window_of_node(*id);
            crate::windows::peek(k, |w| w.full_res_images).unwrap_or(false)
        })
        .map(|(_, key)| key)
        .collect()
}

fn keep_full(source: &ImageSource, image: &gpui::RenderImage) -> bool {
    let Some(key) = source_key(source) else {
        return true;
    };
    RESIDENCY.with(|r| r.borrow_mut().keep_full.insert(key));
    if image.is_reduced() {
        crate::resample::want_full(image.id);
        return false;
    }
    true
}

#[cfg(test)]
fn take_evictions() -> Vec<ImageSource> {
    EVICT.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

pub fn set_asset_base(dir: PathBuf) {
    BASE.with(|b| *b.borrow_mut() = Some(dir));
}

pub fn asset_base() -> Option<PathBuf> {
    BASE.with(|b| b.borrow().clone())
}

/// Makes relative `src`s resolve out of `lookup` instead of the disk; set once at boot, on the
/// render thread, by a packaged exe.
pub fn set_asset_store(lookup: AssetLookup) {
    STORE.with(|s| s.set(Some(lookup)));
}

/// The bytes of one of the app's own files by URL path (`/games/pong/index.html`): from the package,
/// or off disk under the app root (or its `public/`) in a dev host.
pub fn read_app_asset(src: &str) -> Option<Vec<u8>> {
    let key = sk_payload::content_key(src)?;
    if let Some(store) = asset_store() {
        return store(&key).map(<[u8]>::to_vec);
    }
    if let Some(path) = dev_asset_file(src) {
        return std::fs::read(path).ok();
    }
    let base = asset_base()?;
    let relative = key.replace('/', std::path::MAIN_SEPARATOR_STR);
    std::fs::read(base.join(&relative))
        .or_else(|_| std::fs::read(base.join("public").join(&relative)))
        .ok()
}

pub fn asset_store() -> Option<AssetLookup> {
    STORE.with(Cell::get)
}

fn stored(tag: &str, src: &str) -> Option<Result<(String, &'static [u8]), String>> {
    let store = asset_store()?;
    Some(match sk_payload::content_key_checked(src) {
        Ok(key) => match store(&key) {
            Some(bytes) => Ok((key, bytes)),
            None => Err(format!(
                "<{tag} src=\"{src}\">: {key:?} is not in this app's package — a packaged exe carries \
                 only the files its sk.config.json \"assets\" names (default: the assets/ directory). \
                 A browser would have answered 404; the element is broken."
            )),
        },
        Err(KeyError::EscapesRoot) => Err(format!(
            "<{tag} src=\"{src}\">: the path climbs out of the app root, and a packaged app has \
             nothing outside its package."
        )),
        Err(_) => Err(format!("<{tag} src=\"{src}\">: names no file inside the app — nothing to load.")),
    })
}

pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    RESOLVED.with(|m| {
        let mut map = m.borrow_mut();
        let dead: Vec<u32> = map.keys().copied().filter(|&id| !live(id)).collect();
        let gone: Vec<Memo> = dead.iter().filter_map(|id| map.remove(id)).collect();
        for g in gone {
            release(&map, g);
        }
    });
    BACKGROUNDS.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

thread_local! {
    /// `background-image: url()` sources per node as `(url, source)`; a node may carry several layers.
    static BACKGROUNDS: RefCell<HashMap<u32, Vec<(std::rc::Rc<str>, Option<ImageSource>)>>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn background_source(id: u32, url: &str) -> Option<ImageSource> {
    BACKGROUNDS.with(|m| {
        let mut m = m.borrow_mut();
        let list = m.entry(id).or_default();
        if let Some((_, source)) = list.iter().find(|(u, _)| &**u == url) {
            return source.clone();
        }
        let source = resolve(url);
        note_known(source.as_ref());
        // An animated sprite sheet swapping URLs must not grow this without bound.
        if list.len() >= 8 {
            list.clear();
        }
        list.push((std::rc::Rc::from(url), source.clone()));
        source
    })
}

pub(crate) fn source_for(id: u32, src: &str) -> Option<ImageSource> {
    RESOLVED.with(|m| {
        let mut map = m.borrow_mut();
        if let Some(memo) = map.get(&id) {
            if memo.src == src {
                return memo.source.clone();
            }
        }
        let source = resolve(src);
        note_known(source.as_ref());
        let blob = src
            .trim()
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("blob:"));
        let old = map.insert(
            id,
            Memo {
                src: src.to_string(),
                source: source.clone(),
                blob,
            },
        );
        if let Some(old) = old {
            release(&map, old);
        }
        source
    })
}

// `load` is a header read, not a decode: the probe must not hold a second full-size copy of a photo
// a script only wanted to measure. An http(s) image waits on gpui's own decode and shares it.

thread_local! {
    static LOADS: RefCell<std::collections::HashSet<u32>> = RefCell::new(std::collections::HashSet::new());
}

pub(crate) fn note_src(id: u32) {
    LOADS.with(|l| l.borrow_mut().insert(id));
}

#[derive(Debug, PartialEq)]
pub(crate) enum Probe {
    Pending,
    Loaded(f64, f64),
    Failed,
}

pub(crate) fn tick(window: &mut gpui::Window, cx: &mut gpui::App) {
    let pending: Vec<u32> = LOADS.with(|l| l.borrow().iter().copied().collect());
    for id in pending {
        let Some(src) =
            crate::tree::read_tree(|t| t.node(id).map(|_| t.prop(id, "src").map(str::to_string)))
        else {
            LOADS.with(|l| l.borrow_mut().remove(&id));
            continue;
        };
        let Some(src) = src else {
            LOADS.with(|l| l.borrow_mut().remove(&id));
            continue;
        };
        let probe = probe(id, &src, cx);
        let (event, a, b) = match probe {
            Probe::Pending => continue,
            Probe::Loaded(w, h) => (crate::tree::EV_LOAD, w, h),
            Probe::Failed => (crate::tree::EV_ERROR, 0.0, 0.0),
        };
        LOADS.with(|l| l.borrow_mut().remove(&id));
        let payload = crate::EventPayload {
            a,
            b,
            key: Some(&src),
            ..Default::default()
        };
        crate::render::js_dispatch_app(id, event, &payload, window, cx);
    }
}

pub(crate) fn probe(id: u32, src: &str, cx: &mut gpui::App) -> Probe {
    // HTML: an empty `src` fires `error` (it is not "no src").
    if src.trim().is_empty() {
        return Probe::Failed;
    }
    let svg_named = names_svg(src);
    match source_for(id, src) {
        None => Probe::Failed,
        Some(ImageSource::Resource(Resource::Uri(uri))) => {
            match cx.fetch_asset::<gpui::ImgResourceLoader>(&Resource::Uri(uri)) {
                None => Probe::Pending,
                Some(Err(_)) => Probe::Failed,
                Some(Ok(image)) => {
                    let size = image.natural_size(0);
                    // gpui rasterises an SVG at twice its size for smoothness.
                    let scale = if svg_named {
                        gpui::SMOOTH_SVG_SCALE_FACTOR as f64
                    } else {
                        1.0
                    };
                    Probe::Loaded(size.width.0 as f64 / scale, size.height.0 as f64 / scale)
                }
            }
        }
        Some(ImageSource::Resource(Resource::Path(path))) => file_natural_size(path.as_ref()),
        Some(ImageSource::Resource(Resource::Embedded(key))) => {
            match asset_store().and_then(|store| store(key.as_ref())) {
                Some(bytes) => natural_size(bytes),
                None => Probe::Failed,
            }
        }
        Some(ImageSource::Image(image)) => natural_size(&image.bytes),
        Some(_) => Probe::Failed,
    }
}

pub(crate) fn names_svg(src: &str) -> bool {
    let path = src.split(['?', '#']).next().unwrap_or(src).trim();
    path.len() > 4
        && path.is_char_boundary(path.len() - 4)
        && path[path.len() - 4..].eq_ignore_ascii_case(".svg")
}

/// A file's natural size: only the header of a raster (a picked photo is measured, not loaded); an
/// SVG is read whole.
fn file_natural_size(path: &Path) -> Probe {
    let Ok(reader) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()) else {
        return Probe::Failed;
    };
    if reader.format().is_some() {
        return upright_size(reader);
    }
    match std::fs::read(path) {
        Ok(bytes) => svg_size(&bytes),
        Err(_) => Probe::Failed,
    }
}

/// The natural size a format's header declares: the raster formats gpui decodes (sniffed through the
/// same `image` crate, so the set cannot drift), or an SVG's own size.
pub(crate) fn natural_size(bytes: &[u8]) -> Probe {
    if let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() {
        if reader.format().is_some() {
            return upright_size(reader);
        }
    }
    svg_size(bytes)
}

/// A raster's natural size as painted: EXIF orientation applied (`image-orientation: from-image`, as
/// gpui's decoder and `drawImage` do), so a sideways phone photo measures upright.
fn upright_size<R: std::io::BufRead + std::io::Seek>(reader: image::ImageReader<R>) -> Probe {
    use image::metadata::Orientation;
    use image::ImageDecoder;
    let Ok(mut decoder) = reader.into_decoder() else {
        return Probe::Failed;
    };
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return Probe::Failed;
    }
    let turned = matches!(
        decoder.orientation(),
        Ok(Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH)
    );
    if turned {
        Probe::Loaded(h as f64, w as f64)
    } else {
        Probe::Loaded(w as f64, h as f64)
    }
}

fn svg_size(bytes: &[u8]) -> Probe {
    match usvg::Tree::from_data(bytes, &usvg::Options::default()) {
        Ok(tree) => Probe::Loaded(tree.size().width() as f64, tree.size().height() as f64),
        Err(_) => Probe::Failed,
    }
}

// `drawImage` pixels: http(s) uses gpui's decode (BGRA, SVG at twice size, halved back); everything
// else is read and decoded here, synchronously, since gpui cannot be asked without starting a load.

pub(crate) enum Pixels {
    Ready {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    NotYet,
    Broken(String),
}

pub(crate) fn pixels(id: u32, cx: Option<&mut gpui::App>) -> Pixels {
    let Some(raw) = crate::tree::read_tree(|t| t.prop(id, "src").map(str::to_string)) else {
        return Pixels::NotYet;
    };
    let src = raw.trim();
    if src.is_empty() {
        return Pixels::NotYet;
    }
    if matches!(scheme_of(src).as_str(), "http" | "https") {
        let Some(cx) = cx else { return Pixels::NotYet };
        let source = source_for(id, &raw);
        let Some(Ok(image)) = (match &source {
            Some(ImageSource::Resource(r @ Resource::Uri(_))) => {
                cx.fetch_asset::<gpui::ImgResourceLoader>(r)
            }
            _ => {
                return Pixels::Broken(format!("<img src=\"{}\">: not a URL gpui loads", head(src)))
            }
        }) else {
            return Pixels::NotYet;
        };
        // A canvas read: a reduced copy must not stand in; if one does, bring the full decode back
        // (NotYet meanwhile).
        if let Some(source) = &source {
            if !keep_full(source, &image) {
                return Pixels::NotYet;
            }
        }
        let size = image.size(0);
        let (w, h) = (size.width.0.max(0) as u32, size.height.0.max(0) as u32);
        let Some(bgra) = image
            .as_bytes(0)
            .filter(|b| w > 0 && h > 0 && b.len() >= (w * h * 4) as usize)
        else {
            return Pixels::Broken(format!(
                "<img src=\"{}\">: the decoded image has no pixels",
                head(src)
            ));
        };
        let mut rgba = bgra[..(w * h * 4) as usize].to_vec();
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        if names_svg(src) {
            let (rgba, w, h) = halve(&rgba, w, h);
            return Pixels::Ready {
                width: w,
                height: h,
                rgba,
            };
        }
        return Pixels::Ready {
            width: w,
            height: h,
            rgba,
        };
    }
    let bytes = match read_bytes("img", src) {
        Ok(b) => b,
        Err(why) => return Pixels::Broken(why),
    };
    match redjs::webapi::image::decode(&bytes, None) {
        Some(b) => Pixels::Ready {
            width: b.width,
            height: b.height,
            rgba: b.rgba,
        },
        None => Pixels::Broken(format!(
            "<img src=\"{}\">: the bytes are not an image this build decodes",
            head(src)
        )),
    }
}

/// Straight-alpha RGBA halved by 2×2 averaging weighted by alpha: undoes gpui's SVG raster at twice
/// the natural size.
fn halve(rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (ow, oh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let (mut c, mut a) = ([0u32; 3], 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                let p = &rgba[((sy * w + sx) * 4) as usize..][..4];
                let pa = p[3] as u32;
                for i in 0..3 {
                    c[i] += p[i] as u32 * pa;
                }
                a += pa;
            }
            let o = &mut out[((y * ow + x) * 4) as usize..][..4];
            if a > 0 {
                for i in 0..3 {
                    o[i] = ((c[i] + a / 2) / a) as u8;
                }
            }
            o[3] = ((a + 2) / 4) as u8;
        }
    }
    (out, ow, oh)
}

/// The raw file bytes behind a `src`, for elements that parse the document themselves (`<lottie>`).
pub(crate) fn read_bytes(tag: &str, src: &str) -> Result<Vec<u8>, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err(format!(
            "<{tag} src=\"\">: the src is empty — nothing to load"
        ));
    }
    if scheme_of(src) == "data" {
        return data_bytes(tag, src);
    }
    if scheme_of(src) == "blob" {
        return blob_bytes(tag, src, blob_target(src));
    }
    if scheme_of(src).is_empty() {
        if let Some(hit) = stored(tag, src) {
            return hit.map(|(_, bytes)| bytes.to_vec());
        }
    }
    let path = resolve_path(tag, src)?;
    read_file(tag, src, &path)
}

/// What [`read_bytes`] reads without reading a file yet: in-memory bytes (`data:`, `blob:`, packaged
/// asset), or a file path for a caller that reads it on another thread ([`read_file`]).
pub(crate) enum Readable {
    Bytes(Vec<u8>),
    File(PathBuf),
}

pub(crate) fn readable(tag: &str, src: &str) -> Result<Readable, String> {
    let src = src.trim();
    let scheme = scheme_of(src);
    if src.is_empty() || scheme == "data" || scheme == "blob" {
        return read_bytes(tag, src).map(Readable::Bytes);
    }
    if scheme.is_empty() {
        if let Some(hit) = stored(tag, src) {
            return hit.map(|(_, bytes)| Readable::Bytes(bytes.to_vec()));
        }
    }
    resolve_path(tag, src).map(Readable::File)
}

pub(crate) fn read_file(tag: &str, src: &str, path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| {
        format!(
            "<{tag} src=\"{src}\">: could not read {} — {e}",
            path.display()
        )
    })
}

/// The file PATH behind a `src`, for `<video>`: the OS owns the read-ahead, so a long clip never
/// becomes a `Vec<u8>`.
pub(crate) fn resolve_path(tag: &str, src: &str) -> Result<PathBuf, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err(format!(
            "<{tag} src=\"\">: the src is empty — nothing to load"
        ));
    }
    match scheme_of(src).as_str() {
        "data" => Err(format!(
            "<{tag} src=\"data:…\">: this element loads by PATH, and a data: URI has none. \
             (`<video>` takes them — see `resolve_media`.)"
        )),
        "http" | "https" => Err(format!(
            "<{tag} src=\"{src}\">: this element loads by PATH, and an http(s) URL is not one. \
             (`<img>` and `<video>` take them — see `resolve` and `resolve_media`.)"
        )),
        "file" => {
            file_url(src).ok_or_else(|| format!("<{tag} src=\"{src}\">: malformed file: URL"))
        }
        // A blob: URL of a whole picked file IS a path; a slice or an in-memory Blob is not one.
        "blob" => match blob_target(src) {
            Some(t @ BlobTarget::Path { range: None, .. }) => {
                check_snapshot(tag, src, &t)?;
                let BlobTarget::Path { path, .. } = t else {
                    unreachable!()
                };
                Ok(path)
            }
            Some(_) => Err(format!(
                "<{tag} src=\"{src}\">: this element loads by PATH, and this blob: URL names bytes \
                 in memory or a slice of a file, which have none."
            )),
            None => Err(unknown_blob(tag, src)),
        },
        s if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()) => Ok(PathBuf::from(src)),
        "" => Ok(relative_to_base(tag, src)),
        other => Err(format!(
            "<{tag} src=\"{src}\">: the `{other}:` URL scheme is not supported — use a file \
             path or a data: URI."
        )),
    }
}

/// The [`sk_video::MediaSource`] behind a `<video src>`: a path, an http(s) URL verbatim (the
/// platform streams it), or bytes for `data:`.
pub(crate) fn resolve_media(tag: &str, src: &str) -> Result<sk_video::MediaSource, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err(format!(
            "<{tag} src=\"\">: the src is empty — nothing to load"
        ));
    }
    match scheme_of(src).as_str() {
        "data" => {
            // The media type is the platform's only hint about the bytes.
            let mime = src
                .get(5..)
                .unwrap_or_default()
                .split_once(',')
                .map(|(meta, _)| {
                    meta.split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                })
                .unwrap_or_default();
            let data = data_bytes(tag, src)?;
            if data.is_empty() {
                return Err(format!(
                    "<{tag} src=\"{}…\">: the data: URI decoded to zero bytes",
                    head(src)
                ));
            }
            Ok(sk_video::MediaSource::Bytes {
                mime,
                data: std::sync::Arc::new(data),
            })
        }
        "http" | "https" => Ok(sk_video::MediaSource::Uri(src.to_string())),
        "blob" => blob_media(tag, src, blob_target(src)),
        "" if asset_store().is_some() => {
            let (key, bytes) = stored(tag, src).expect("asset_store() is set")?;
            Ok(sk_video::MediaSource::StaticBytes {
                mime: sk_payload::media_type_for(&key).to_string(),
                data: bytes,
            })
        }
        _ => resolve_path(tag, src).map(sk_video::MediaSource::Path),
    }
}

fn scheme_of(src: &str) -> String {
    src.split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default()
}

fn data_bytes(tag: &str, src: &str) -> Result<Vec<u8>, String> {
    let body = src.get(5..).unwrap_or_default();
    let Some((meta, payload)) = body.split_once(',') else {
        return Err(format!(
            "<{tag} src=\"{}…\">: malformed data: URL (no comma)",
            head(src)
        ));
    };
    if meta
        .to_ascii_lowercase()
        .split(';')
        .any(|p| p.trim() == "base64")
    {
        base64_decode(payload).ok_or_else(|| {
            format!(
                "<{tag} src=\"{}…\">: data: URL payload is not valid base64",
                head(src)
            )
        })
    } else {
        Ok(percent_decode_bytes(payload))
    }
}

fn warn(msg: String) {
    thread_local! {
        static SEEN: RefCell<std::collections::HashSet<String>> =
            RefCell::new(std::collections::HashSet::new());
    }
    SEEN.with(|s| {
        if s.borrow_mut().insert(msg.clone()) {
            eprintln!("[skathi compat] {msg}");
            crate::log::record("advisory", msg);
        }
    });
}

fn resolve(src: &str) -> Option<ImageSource> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    let lower_scheme = src
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .unwrap_or_default();

    match lower_scheme.as_str() {
        "data" => data_url(src),
        // Fetched asynchronously by gpui, so the element shows `alt` for a frame or two. NOT
        // `ImageSource::from(&str)`: gpui's `is_uri` classifies a bare relative path as `Embedded`.
        "http" | "https" => Some(ImageSource::Resource(Resource::Uri(src.to_string().into()))),
        "blob" => blob_image(src, blob_target(src)),
        "file" => file_url(src).map(path_source),
        s if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()) => {
            Some(path_source(PathBuf::from(src)))
        }
        "" => match stored("img", src) {
            // gpui reads `Embedded` through the app's `AssetSource` (the payload) off the render
            // thread and caches the decode by key.
            Some(Ok((key, _))) => Some(ImageSource::Resource(Resource::Embedded(key.into()))),
            Some(Err(msg)) => {
                warn(msg);
                None
            }
            None => Some(path_source(relative_to_base("img", src))),
        },
        other => {
            warn(format!(
                "<img src=\"{src}\">: the `{other}:` URL scheme is not supported — use a file \
                 path, a data: URI, or (once an HTTP client is wired) http(s)."
            ));
            None
        }
    }
}

fn path_source(p: PathBuf) -> ImageSource {
    // Explicitly `Resource::Path`: `ImageSource::from(&str)` would parse `C:` as a URL scheme.
    ImageSource::Resource(Resource::Path(p.into()))
}

/// Joins a relative or root-relative `src` onto the app root through [`content_key`], as `fetch()` does.
fn relative_to_base(tag: &str, src: &str) -> PathBuf {
    if let Some(path) = dev_asset_file(src) {
        return path;
    }
    let base = match asset_base() {
        Some(b) => b,
        None => {
            warn(
                "<img src>: no app root is published, so relative image paths resolve against the \
                 process working directory. Set SK_APP_ROOT, or run the dev host (which derives \
                 it from the manifest entry)."
                    .into(),
            );
            std::env::current_dir().unwrap_or_default()
        }
    };
    let sep = std::path::MAIN_SEPARATOR_STR;
    match sk_payload::content_key_checked(src) {
        Ok(key) => base.join(key.replace('/', sep)),
        Err(why) => {
            if why == KeyError::EscapesRoot {
                warn(format!(
                    "<{tag} src=\"{src}\"> climbs out of the app root ({}) — it resolves in a dev host, \
                     but a packaged exe carries only the app's own files, so there it will be broken.",
                    base.display()
                ));
            }
            let rel = src.split(['?', '#']).next().unwrap_or_default();
            let rel = rel.trim_start_matches(['/', '\\']);
            normalize(&base.join(rel.replace('/', sep)))
        }
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn file_url(src: &str) -> Option<PathBuf> {
    file_url_for(src, cfg!(windows))
}

fn file_url_for(src: &str, windows: bool) -> Option<PathBuf> {
    let rest = src.get(5..)?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let decoded = percent_decode(rest);
    if !windows {
        let path = match decoded.find('/') {
            Some(at) => &decoded[at..],
            None => decoded.as_str(),
        };
        return Some(PathBuf::from(path));
    }
    let decoded = decoded.replace('/', "\\");
    // "///C:/x" → "\C:\x": drop the empty authority's slash before a drive.
    let trimmed = decoded.strip_prefix('\\').unwrap_or(&decoded);
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' {
        return Some(PathBuf::from(trimmed));
    }
    Some(PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    String::from_utf8_lossy(&percent_decode_bytes(s)).into_owned()
}

/// The bytes a percent-encoded string stands for (a `data:` payload is bytes, not text).
fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// blob: URLs. A whole picked file is handed on as its path, like an absolute `src`; a path-backed
// URL fails when the file's size or mtime changed since it was picked.

fn blob_target(src: &str) -> Option<BlobTarget> {
    JsRuntime::try_with(|rt| redjs::webapi::url::resolve_blob_url(rt.raw(), src)).flatten()
}

fn unknown_blob(tag: &str, src: &str) -> String {
    format!(
        "<{tag} src=\"{src}\">: this blob: URL is not in the blob URL store — it was revoked \
         (URL.revokeObjectURL), or was never created in this window."
    )
}

fn check_snapshot(tag: &str, src: &str, t: &BlobTarget) -> Result<(), String> {
    if t.snapshot_holds() {
        return Ok(());
    }
    let path = match t {
        BlobTarget::Path { path, .. } => path.display().to_string(),
        BlobTarget::Bytes { .. } => String::new(),
    };
    Err(format!(
        "<{tag} src=\"{src}\">: {path} changed (or is gone) since its File was made, so the load \
         fails — as a browser's does (NotReadableError). Pick the file again for its new contents."
    ))
}

/// The bytes a blob: URL names: a copy of the in-memory window, or read from the file for a path-backed one.
fn blob_bytes(tag: &str, src: &str, target: Option<BlobTarget>) -> Result<Vec<u8>, String> {
    match target {
        None => Err(unknown_blob(tag, src)),
        Some(BlobTarget::Bytes {
            bytes, offset, len, ..
        }) => match bytes.get(offset..offset + len) {
            Some(b) => Ok(b.to_vec()),
            None => Err(format!(
                "<{tag} src=\"{src}\">: the blob: URL's window lies outside its bytes"
            )),
        },
        Some(t @ BlobTarget::Path { .. }) => {
            check_snapshot(tag, src, &t)?;
            let BlobTarget::Path {
                path, range, size, ..
            } = t
            else {
                unreachable!()
            };
            let (offset, len) = range.unwrap_or((0, size));
            read_window(&path, offset, len).map_err(|e| {
                format!(
                    "<{tag} src=\"{src}\">: could not read {} — {e}",
                    path.display()
                )
            })
        }
    }
}

/// `len` bytes of `path` from `offset`, or an error: never a truncated decode or an OOM abort.
fn read_window(path: &Path, offset: u64, len: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let want = usize::try_from(len)
        .map_err(|_| Error::new(ErrorKind::OutOfMemory, "too large to hold in memory"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(want).map_err(|_| {
        Error::new(
            ErrorKind::OutOfMemory,
            format!("{len} bytes could not be held in memory"),
        )
    })?;
    f.take(len).read_to_end(&mut out)?;
    if out.len() != want {
        return Err(Error::new(
            ErrorKind::UnexpectedEof,
            format!(
                "the file ended after {} of the {len} bytes the blob: URL names",
                out.len()
            ),
        ));
    }
    Ok(out)
}

fn blob_image(src: &str, target: Option<BlobTarget>) -> Option<ImageSource> {
    if let Some(t @ BlobTarget::Path { range: None, .. }) = &target {
        if let Err(msg) = check_snapshot("img", src, t) {
            warn(msg);
            return None;
        }
        let BlobTarget::Path {
            path, size, mtime, ..
        } = t
        else {
            unreachable!()
        };
        note_blob_path(path, (*size, *mtime));
        return Some(path_source(path.clone()));
    }
    let mime = target
        .as_ref()
        .map(|t| t.mime().to_ascii_lowercase())
        .unwrap_or_default();
    let bytes = match blob_bytes("img", src, target) {
        Ok(b) => b,
        Err(msg) => {
            warn(msg);
            return None;
        }
    };
    let Some(format) = sniff(&bytes).or_else(|| image_format(&mime)) else {
        warn(format!(
            "<img src=\"{src}\">: the Blob is not an image this renderer decodes (type {mime:?}) — png, \
             jpeg, webp, gif, bmp, tiff, ico and svg+xml do"
        ));
        return None;
    };
    Some(ImageSource::Image(std::sync::Arc::new(Image::from_bytes(
        format, bytes,
    ))))
}

/// `<video src="blob:…">`: a whole picked file is a path (the OS streams it); an in-memory Blob is
/// its own `Arc`.
fn blob_media(
    tag: &str,
    src: &str,
    target: Option<BlobTarget>,
) -> Result<sk_video::MediaSource, String> {
    match target {
        None => Err(unknown_blob(tag, src)),
        Some(t @ BlobTarget::Path { range: None, .. }) => {
            check_snapshot(tag, src, &t)?;
            let BlobTarget::Path { path, .. } = t else {
                unreachable!()
            };
            Ok(sk_video::MediaSource::Path(path))
        }
        Some(BlobTarget::Bytes {
            bytes,
            offset: 0,
            len,
            mime,
        }) if len == bytes.len() => {
            if len == 0 {
                return Err(format!("<{tag} src=\"{src}\">: the Blob is empty"));
            }
            Ok(sk_video::MediaSource::Bytes {
                mime: mime.to_ascii_lowercase(),
                data: bytes,
            })
        }
        Some(t) => {
            let mime = t.mime().to_ascii_lowercase();
            let data = blob_bytes(tag, src, Some(t))?;
            if data.is_empty() {
                return Err(format!("<{tag} src=\"{src}\">: the Blob is empty"));
            }
            Ok(sk_video::MediaSource::Bytes {
                mime,
                data: std::sync::Arc::new(data),
            })
        }
    }
}

/// `data:[<mediatype>][;base64],<data>` (RFC 2397).
fn data_url(src: &str) -> Option<ImageSource> {
    let body = src.get(5..)?;
    let (meta, payload) = match body.split_once(',') {
        Some(pair) => pair,
        None => {
            warn(format!(
                "<img src=\"{}…\">: malformed data: URL (no comma)",
                head(src)
            ));
            return None;
        }
    };
    let meta_lower = meta.to_ascii_lowercase();
    let is_base64 = meta_lower.split(';').any(|p| p.trim() == "base64");
    let mime = meta_lower
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    let bytes = if is_base64 {
        match base64_decode(payload) {
            Some(b) => b,
            None => {
                warn(format!(
                    "<img src=\"{}…\">: data: URL payload is not valid base64",
                    head(src)
                ));
                return None;
            }
        }
    } else {
        percent_decode_bytes(payload)
    };

    let format = image_format(&mime).or_else(|| sniff(&bytes));
    let Some(format) = format else {
        warn(format!(
            "<img src=\"{}…\">: unsupported data: URL media type {mime:?} — png, jpeg, webp, gif, \
             bmp, tiff, ico and svg+xml decode",
            head(src)
        ));
        return None;
    };
    Some(ImageSource::Image(std::sync::Arc::new(Image::from_bytes(
        format, bytes,
    ))))
}

/// A data: URL IS its content — never echo the whole thing into the log.
fn head(src: &str) -> String {
    src.chars().take(40).collect()
}

fn image_format(mime: &str) -> Option<ImageFormat> {
    Some(match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::Webp,
        "image/gif" => ImageFormat::Gif,
        "image/svg+xml" => ImageFormat::Svg,
        "image/bmp" | "image/x-bmp" => ImageFormat::Bmp,
        "image/tiff" | "image/tif" => ImageFormat::Tiff,
        "image/x-icon" | "image/vnd.microsoft.icon" | "image/ico" => ImageFormat::Ico,
        _ => return None,
    })
}

fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    let b = bytes;
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(ImageFormat::Png)
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if b.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if b.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else if b.starts_with(b"<svg") || b.starts_with(b"<?xml") {
        Some(ImageFormat::Svg)
    } else {
        None
    }
}

/// Standard base64 with optional padding; whitespace is skipped.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' | 0x0c => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Y:\a\b` on Windows, `/Y/a/b` elsewhere.
    fn at(windows: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(windows)
        } else {
            PathBuf::from(windows.replacen("Y:", "/Y", 1).replace('\\', "/"))
        }
    }

    fn as_path(s: Option<ImageSource>) -> Option<PathBuf> {
        match s {
            Some(ImageSource::Resource(Resource::Path(p))) => Some(p.to_path_buf()),
            _ => None,
        }
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(w, h)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn an_img_hands_a_canvas_its_pixels() {
        let dir = std::env::temp_dir().join(format!("sk-img-pixels-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        set_asset_base(dir.clone());
        let good = dir.join("a.png");
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&good, out.into_inner()).unwrap();
        let bad = dir.join("b.png");
        std::fs::write(&bad, b"not a png").unwrap();
        let node = |src: Option<&str>| {
            crate::tree::with_tree(|t| {
                let tag = t.intern("img");
                let id = t.create_element(tag);
                let key = t.intern("src");
                t.set_prop(id, key, src.map(str::to_string));
                id
            })
        };
        match pixels(node(Some("a.png")), None) {
            Pixels::Ready {
                width,
                height,
                rgba,
            } => {
                assert_eq!((width, height), (3, 2));
                assert_eq!(&rgba[..4], &[10, 20, 30, 255]);
            }
            _ => panic!("a PNG file did not decode"),
        }
        let turned = dir.join("c.jpg");
        std::fs::write(&turned, sideways_jpeg()).unwrap();
        assert_eq!(file_natural_size(&turned), Probe::Loaded(16.0, 32.0));
        assert_eq!(natural_size(&sideways_jpeg()), Probe::Loaded(16.0, 32.0));
        assert!(matches!(
            pixels(node(Some("c.jpg")), None),
            Pixels::Ready {
                width: 16,
                height: 32,
                ..
            }
        ));
        assert!(matches!(
            pixels(node(Some("b.png")), None),
            Pixels::Broken(_)
        ));
        assert!(matches!(
            pixels(node(Some("https://cdn.example/a.png")), None),
            Pixels::NotYet
        ));
        assert!(matches!(pixels(node(None), None), Pixels::NotYet));
        assert!(matches!(pixels(node(Some("  ")), None), Pixels::NotYet));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A 32×16 JPEG with Exif orientation 6: upright it is 16×32.
    fn sideways_jpeg() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(32, 16, image::Rgb([200, 0, 0]))
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        let jpeg = out.into_inner();
        let tiff = [
            0x4D, 0x4D, 0, 0x2A, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0,
            0, 0,
        ];
        let len = (2 + 6 + tiff.len()) as u16;
        let mut with = jpeg[..2].to_vec();
        with.extend_from_slice(&[0xFF, 0xE1, (len >> 8) as u8, len as u8]);
        with.extend_from_slice(b"Exif\0\0");
        with.extend_from_slice(&tiff);
        with.extend_from_slice(&jpeg[2..]);
        with
    }

    #[test]
    fn halving_weighs_colour_by_alpha() {
        let px = [
            [255, 0, 0, 255],
            [0, 0, 255, 0],
            [255, 0, 0, 255],
            [0, 0, 255, 0],
        ]
        .concat();
        let (out, w, h) = halve(&px, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![255, 0, 0, 128]);
        let (out, w, h) = halve(&[9, 9, 9, 255].repeat(3), 3, 1);
        assert_eq!((w, h, out), (1, 1, vec![9, 9, 9, 255]));
    }

    #[test]
    fn the_probe_reads_the_natural_size_off_the_header() {
        assert_eq!(natural_size(&png(7, 3)), Probe::Loaded(7.0, 3.0));
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="24"></svg>"#;
        assert_eq!(natural_size(svg), Probe::Loaded(40.0, 24.0));
        assert_eq!(natural_size(b"not an image"), Probe::Failed);
        assert_eq!(natural_size(&png(4, 4)[..12]), Probe::Failed);
    }

    #[test]
    fn a_file_is_measured_from_its_header() {
        let dir = std::env::temp_dir().join(format!("sk-img-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raster = dir.join("a.png");
        std::fs::write(&raster, png(9, 4)).unwrap();
        assert_eq!(file_natural_size(&raster), Probe::Loaded(9.0, 4.0));
        let svg = dir.join("b.svg");
        std::fs::write(
            &svg,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"/>"#,
        )
        .unwrap();
        assert_eq!(file_natural_size(&svg), Probe::Loaded(3.0, 2.0));
        assert_eq!(file_natural_size(&dir.join("missing.png")), Probe::Failed);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_svg_url_is_known_by_its_path() {
        assert!(names_svg("https://cdn.example/icons/a.svg"));
        assert!(names_svg("https://cdn.example/a.SVG?v=2#frag"));
        assert!(!names_svg("https://cdn.example/a.png?x=.svg"));
        assert!(!names_svg("svg"));
    }

    #[test]
    fn relative_paths_resolve_against_the_app_root() {
        set_asset_base(at("Y:\\skathi\\apps\\imgperf"));
        for src in ["assets/a.png", "./assets/a.png", "/assets/a.png"] {
            assert_eq!(
                as_path(resolve(src)),
                Some(at("Y:\\skathi\\apps\\imgperf\\assets\\a.png")),
                "{src}"
            );
        }
        assert_eq!(
            as_path(resolve("../hello/assets/a.png")),
            Some(at("Y:\\skathi\\apps\\hello\\assets\\a.png"))
        );
    }

    #[test]
    fn absolute_windows_paths_are_paths_not_uris() {
        set_asset_base(at("Y:\\skathi\\apps\\imgperf"));
        assert_eq!(
            as_path(resolve("C:/pics/a.png")),
            Some(PathBuf::from("C:/pics/a.png"))
        );
        assert_eq!(
            as_path(resolve("C:\\pics\\a.png")),
            Some(PathBuf::from("C:\\pics\\a.png"))
        );
    }

    #[test]
    #[cfg(windows)]
    fn file_urls_decode_to_paths() {
        assert_eq!(
            as_path(resolve("file:///C:/my%20pics/a.png")),
            Some(PathBuf::from("C:\\my pics\\a.png"))
        );
    }

    #[test]
    fn file_urls_keep_their_path_off_windows() {
        let unix = |src| file_url_for(src, false).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(
            unix("file:///home/u/my%20pics/a.png").as_deref(),
            Some("/home/u/my pics/a.png")
        );
        assert_eq!(
            unix("file://localhost/home/u/a.png").as_deref(),
            Some("/home/u/a.png")
        );
        assert_eq!(unix("file:/home/u/a.png").as_deref(), Some("/home/u/a.png"));
        let windows = |src| file_url_for(src, true).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(
            windows("file:///C:/x/a.png").as_deref(),
            Some("C:\\x\\a.png")
        );
    }

    #[test]
    fn http_becomes_a_uri_resource_for_gpuis_asset_loader() {
        assert!(matches!(
            resolve("https://example.com/a.png"),
            Some(ImageSource::Resource(Resource::Uri(_)))
        ));
        assert!(matches!(
            resolve("http://example.com/a.png"),
            Some(ImageSource::Resource(Resource::Uri(_)))
        ));
        assert!(matches!(
            resolve("assets/a.png"),
            Some(ImageSource::Resource(Resource::Path(_)))
        ));
    }

    #[test]
    fn video_sources_resolve_to_the_shape_the_platform_wants() {
        use sk_video::MediaSource;
        assert!(matches!(
            resolve_media("video", "https://example.com/a.mp4"),
            Ok(MediaSource::Uri(u)) if u == "https://example.com/a.mp4"
        ));
        match resolve_media("video", "data:video/mp4;base64,AAAA") {
            Ok(MediaSource::Bytes { mime, data }) => {
                assert_eq!(mime, "video/mp4");
                assert_eq!(&*data, &[0u8, 0, 0]);
            }
            other => panic!("{other:?}"),
        }
        // Not a zero-byte load that fails deep inside Media Foundation.
        assert!(resolve_media("video", "data:video/mp4;base64,").is_err());
        assert!(resolve_media("video", "").is_err());
        assert!(matches!(
            resolve_media("video", "assets/clip.mp4"),
            Ok(MediaSource::Path(_))
        ));
    }

    #[test]
    fn data_urls_decode_and_sniff() {
        const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
        let Some(ImageSource::Image(img)) = resolve(PNG) else {
            panic!("data: URL did not resolve to an image");
        };
        assert_eq!(img.format, ImageFormat::Png);
        assert!(img.bytes.starts_with(&[0x89, b'P', b'N', b'G']));

        let sniffed = resolve("data:;base64,iVBORw0KGgoAAAANSUhEUg==");
        assert!(matches!(sniffed, Some(ImageSource::Image(_))));

        let svg =
            resolve("data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A//www.w3.org/2000/svg%22/%3E");
        let Some(ImageSource::Image(img)) = svg else {
            panic!("percent-encoded data: URL did not resolve");
        };
        assert_eq!(img.format, ImageFormat::Svg);
        assert!(String::from_utf8_lossy(&img.bytes).starts_with("<svg"));

        let Some(ImageSource::Image(img)) = resolve("data:image/png,%89PNG%0D%0A%1A%0A%FF%00")
        else {
            panic!("percent-encoded binary data: URL did not resolve");
        };
        assert_eq!(
            &img.bytes[..],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0x00]
        );
    }

    #[test]
    fn empty_and_unknown_schemes_yield_nothing() {
        assert!(resolve("").is_none());
        assert!(resolve("   ").is_none());
        assert!(resolve("gopher://x/a.png").is_none());
    }

    #[test]
    fn base64_ignores_wrapping_whitespace() {
        assert_eq!(base64_decode("QUJD").unwrap(), b"ABC");
        assert_eq!(base64_decode("QU\n JD\r\n").unwrap(), b"ABC");
        assert_eq!(base64_decode("QQ==").unwrap(), b"A");
        assert!(base64_decode("**").is_none());
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    static FAKE: &[(&str, &[u8])] = &[
        ("assets/a.png", PNG),
        ("assets/clip.mp4", b"mp4bytes"),
        ("assets/s.tgs", b"tgs"),
        ("assets/my pic.png", PNG),
    ];
    fn fake_store(key: &str) -> Option<&'static [u8]> {
        FAKE.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
    /// The store is thread-local; take it away again so a reused thread does not inherit it.
    struct StoreGuard;
    impl Drop for StoreGuard {
        fn drop(&mut self) {
            STORE.with(|s| s.set(None));
        }
    }
    fn packaged() -> StoreGuard {
        set_asset_store(fake_store);
        StoreGuard
    }

    #[test]
    fn a_packaged_app_resolves_relative_srcs_out_of_its_store() {
        let _g = packaged();
        for src in [
            "assets/a.png",
            "./assets/a.png",
            "/assets/a.png",
            "assets/a.png?v=2",
            "assets/x/../a.png",
        ] {
            match resolve(src) {
                Some(ImageSource::Resource(Resource::Embedded(key))) => {
                    assert_eq!(key.as_ref(), "assets/a.png", "{src}")
                }
                other => panic!(
                    "{src}: not an embedded resource (some: {})",
                    other.is_some()
                ),
            }
        }
        assert!(matches!(
            resolve("assets/my%20pic.png"),
            Some(ImageSource::Resource(Resource::Embedded(k))) if k.as_ref() == "assets/my pic.png"
        ));
        // A miss is a broken image (alt text), NEVER the disk — even with a base set.
        set_asset_base(at("Y:\\skathi\\apps\\hello"));
        assert!(resolve("assets/nope.png").is_none());
        assert!(resolve("../hello/assets/a.png").is_none());
        assert!(matches!(
            resolve("C:/pics/a.png"),
            Some(ImageSource::Resource(Resource::Path(_)))
        ));
        assert!(matches!(
            resolve("https://example.com/a.png"),
            Some(ImageSource::Resource(Resource::Uri(_)))
        ));
        assert!(matches!(
            resolve("data:;base64,iVBORw0KGgo="),
            Some(ImageSource::Image(_))
        ));
    }

    #[test]
    fn lottie_and_video_read_the_same_store() {
        use sk_video::MediaSource;
        let _g = packaged();
        assert_eq!(read_bytes("lottie", "assets/s.tgs").unwrap(), b"tgs");
        assert_eq!(read_bytes("lottie", "/assets/s.tgs?x").unwrap(), b"tgs");
        let miss = read_bytes("lottie", "assets/missing.tgs").unwrap_err();
        assert!(miss.contains("not in this app's package"), "{miss}");
        match resolve_media("video", "assets/clip.mp4") {
            Ok(MediaSource::StaticBytes { mime, data }) => {
                assert_eq!(mime, "video/mp4");
                assert_eq!(data, &b"mp4bytes"[..]);
                assert!(
                    std::ptr::eq(data, fake_store("assets/clip.mp4").unwrap()),
                    "mapped package bytes must be borrowed"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(resolve_media("video", "assets/missing.mp4").is_err());
        assert!(matches!(
            resolve_media("video", "https://example.com/a.mp4"),
            Ok(MediaSource::Uri(_))
        ));
    }

    #[test]
    fn a_dev_host_has_no_store_and_joins_the_same_key_onto_the_app_root() {
        assert!(asset_store().is_none());
        set_asset_base(at("Y:\\base"));
        // Browser-shaped: the server never sees the query, and decodes escapes.
        assert_eq!(as_path(resolve("a.png?v=2")), Some(at("Y:\\base\\a.png")));
        assert_eq!(
            as_path(resolve("my%20pic.png")),
            Some(at("Y:\\base\\my pic.png"))
        );
        assert_eq!(
            as_path(resolve("dir\\a.png")),
            Some(at("Y:\\base\\dir\\a.png"))
        );
        // A decoded drive letter must not replace the app root.
        assert_eq!(
            as_path(resolve("C%3A/x.png")),
            Some(at("Y:\\base\\C%3A\\x.png"))
        );
        assert!(matches!(
            resolve_media("video", "assets/clip.mp4"),
            Ok(sk_video::MediaSource::Path(_))
        ));
    }

    /// The memo must follow the `src`, not the node id — Vue reuses ids.
    #[test]
    fn the_per_node_memo_follows_the_src_string() {
        set_asset_base(at("Y:\\base"));
        assert_eq!(as_path(source_for(7, "a.png")), Some(at("Y:\\base\\a.png")));
        assert_eq!(as_path(source_for(7, "b.png")), Some(at("Y:\\base\\b.png")));
    }

    /// A temp file with the target a picked File over it would resolve to; removed on drop.
    struct Picked(PathBuf);

    impl Picked {
        fn new(name: &str, bytes: &[u8]) -> Picked {
            let dir =
                std::env::temp_dir().join(format!("sk-tree-blob-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            Picked(p)
        }

        fn target(&self, range: Option<(u64, u64)>, mime: &str) -> BlobTarget {
            let m = std::fs::metadata(&self.0).unwrap();
            BlobTarget::Path {
                path: self.0.clone(),
                range,
                size: m.len(),
                mtime: m.modified().ok(),
                mime: mime.into(),
            }
        }
    }

    impl Drop for Picked {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_dir(self.0.parent().unwrap());
        }
    }

    const PNG_1PX: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R', 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89,
    ];

    #[test]
    fn a_whole_picked_file_loads_by_path_without_a_copy() {
        let f = Picked::new("pic.png", PNG_1PX);
        let src = "blob:app://redjs/00000000-0000-4000-8000-000000000001";
        assert_eq!(
            as_path(blob_image(src, Some(f.target(None, "image/png")))),
            Some(f.0.clone())
        );
        assert!(matches!(
            blob_media("video", src, Some(f.target(None, "video/mp4"))),
            Ok(sk_video::MediaSource::Path(p)) if p == f.0
        ));
    }

    #[test]
    fn a_slice_or_bytes_in_memory_decode_from_their_bytes() {
        let mut body = b"xx".to_vec();
        body.extend_from_slice(PNG_1PX);
        let f = Picked::new("two.bin", &body);
        let src = "blob:app://redjs/00000000-0000-4000-8000-000000000002";
        let t = f.target(Some((2, PNG_1PX.len() as u64)), "");
        assert!(matches!(
            blob_image(src, Some(t)),
            Some(ImageSource::Image(_))
        ));
        let arc = std::sync::Arc::new(PNG_1PX.to_vec());
        let whole = BlobTarget::Bytes {
            bytes: arc.clone(),
            offset: 0,
            len: arc.len(),
            mime: "video/mp4".into(),
        };
        match blob_media("video", src, Some(whole)) {
            Ok(sk_video::MediaSource::Bytes { data, mime }) => {
                assert!(std::sync::Arc::ptr_eq(&data, &arc));
                assert_eq!(mime, "video/mp4");
            }
            other => panic!("{other:?}"),
        }
        let part = BlobTarget::Bytes {
            bytes: arc.clone(),
            offset: 1,
            len: 3,
            mime: String::new(),
        };
        assert_eq!(blob_bytes("lottie", src, Some(part)).unwrap(), b"PNG");
        let text = BlobTarget::Bytes {
            bytes: std::sync::Arc::new(b"hello".to_vec()),
            offset: 0,
            len: 5,
            mime: "text/plain".into(),
        };
        assert!(blob_image(src, Some(text)).is_none());
    }

    #[test]
    fn a_changed_file_or_an_unknown_url_fails_the_load() {
        let f = Picked::new("snap.png", PNG_1PX);
        let src = "blob:app://redjs/00000000-0000-4000-8000-000000000003";
        let t = f.target(None, "image/png");
        std::fs::write(&f.0, b"a different file now").unwrap();
        assert!(blob_image(src, Some(t.clone())).is_none());
        assert!(blob_media("video", src, Some(t))
            .unwrap_err()
            .contains("changed"));
        assert!(resolve(src).is_none());
        assert!(read_bytes("lottie", src)
            .unwrap_err()
            .contains("blob URL store"));
        assert!(resolve_media("video", src).unwrap_err().contains("revoked"));
    }

    #[test]
    fn a_window_is_read_whole_or_refused() {
        let f = Picked::new("window.bin", b"0123456789");
        assert_eq!(read_window(&f.0, 4, 4).unwrap(), b"4567");
        let short = read_window(&f.0, 8, 10).unwrap_err();
        assert_eq!(short.kind(), std::io::ErrorKind::UnexpectedEof, "{short}");
        let huge = read_window(&f.0, 0, u64::MAX).unwrap_err();
        assert_eq!(huge.kind(), std::io::ErrorKind::OutOfMemory, "{huge}");
    }

    /// A URL a script made in this thread's runtime is found by the render walk's lookup.
    #[test]
    fn a_url_made_by_script_resolves_through_the_runtimes_store() {
        let f = Picked::new("picked.png", PNG_1PX);
        sk_runtime::testing::on_js_thread(move || {
            let eval = |src: &str| -> String {
                JsRuntime::with(|rt| {
                    let v = rt
                        .ctx()
                        .eval(src)
                        .unwrap_or_else(|e| panic!("{src}: {e:?}"));
                    rt.ctx().to_rust_string(v).unwrap()
                })
            };
            let bytes: Vec<String> = PNG_1PX.iter().map(|b| b.to_string()).collect();
            let mem = eval(&format!(
                "URL.createObjectURL(new Blob([new Uint8Array([{}])], {{ type: 'image/png' }}))",
                bytes.join(",")
            ));
            assert!(
                matches!(resolve(&mem), Some(ImageSource::Image(_))),
                "{mem}"
            );
            JsRuntime::with(|rt| unsafe {
                let g = rt.raw();
                let file = redjs::webapi::file::file_from_path(g, &f.0, "").unwrap();
                redjs::webapi::util::define(g, redjs::glue::rjs_global(g), "picked", file, 0);
            });
            let by_path = eval("URL.createObjectURL(picked)");
            assert_eq!(
                as_path(resolve(&by_path)),
                Some(f.0.clone()),
                "a picked file is handed on by path"
            );
            assert!(
                matches!(resolve_media("video", &by_path), Ok(sk_video::MediaSource::Path(p)) if p == f.0)
            );
            let again = eval("URL.createObjectURL(picked)");
            let _ = take_evictions();
            assert!(source_for(901, &mem).is_some());
            assert!(source_for(902, &by_path).is_some());
            assert!(
                source_for(903, &again).is_some(),
                "a second URL of the same File: the same path"
            );
            gc(|id| id != 901 && id != 902);
            let ev = take_evictions();
            assert_eq!(
                ev.len(),
                1,
                "the memory decode goes; the path is still shown by 903"
            );
            assert!(matches!(&ev[0], ImageSource::Image(_)));
            gc(|id| id != 903);
            let ev = take_evictions();
            assert!(
                matches!(&ev[..], [ImageSource::Resource(Resource::Path(p))] if p.as_ref() == f.0),
                "{} queued",
                ev.len()
            );
            eval(&format!("URL.revokeObjectURL('{mem}'); 'ok'"));
            assert!(resolve(&mem).is_none(), "revoked");
        });
    }

    #[test]
    fn a_file_picked_again_after_it_changed_evicts_the_old_decode() {
        let f = Picked::new("again.png", PNG_1PX);
        let first = "blob:app://redjs/00000000-0000-4000-8000-0000000000a1";
        let t1 = f.target(None, "image/png");
        assert!(blob_image(first, Some(t1.clone())).is_some());
        assert!(
            take_evictions().is_empty(),
            "first sight: nothing of it is cached"
        );
        assert!(blob_image(first, Some(t1)).is_some());
        assert!(
            take_evictions().is_empty(),
            "the same file again: the cached decode is right"
        );
        let mut edited = PNG_1PX.to_vec();
        edited.extend([0u8; 16]);
        std::fs::write(&f.0, &edited).unwrap();
        let t2 = f.target(None, "image/png");
        assert!(blob_image(
            "blob:app://redjs/00000000-0000-4000-8000-0000000000a2",
            Some(t2)
        )
        .is_some());
        let ev = take_evictions();
        assert!(
            matches!(&ev[..], [ImageSource::Resource(Resource::Path(p))] if p.as_ref() == f.0),
            "{} queued",
            ev.len()
        );
    }
}
