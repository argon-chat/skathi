//! vn-sticker — the engine behind `<vn-sticker>`: Lottie (`.tgs`) and VP9-alpha
//! WebM stickers and animated emoji, many on screen at once, drawn as cheaply as
//! the frame allows.
//!
//! The element (vn-tree) asks for a source at a size; this crate decodes it off
//! the UI thread and hands back straight-alpha BGRA frames for a `LiveTile`.
//! One decoder per source and size is shared by every place that shows it, all
//! of them on one clock, and small loops are kept compressed so their decoder
//! can be freed.
//!
//! # Ownership
//!
//! * [`video`] — the VP9 track (WebM demux, libvpx, I420+A to BGRA). Its
//!   contract is that module's docs; the video track fills the bodies in.
//! * Everything else (registry, clock, worker, loop cache, budgets, the Lottie
//!   side) is the core's.

pub mod video;
