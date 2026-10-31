//! What a compiled stylesheet keeps in memory, split into the parse (the
//! source text and lightningcss's AST, which the engine's rules borrow their
//! selectors from and the parse cache keeps for the next recompile) and the
//! engine built from it. A measuring tool, not a check (research/reports/m26
//! perf-mem-2.md §1):
//!
//! ```text
//! VN_SHEET=path/to/app.css cargo test -p vn-css --test sheet_memory -- --ignored --nocapture
//! ```
//!
//! Its own test binary, because it counts every allocation through a global
//! allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, Ordering};

struct Counting;

static LIVE: AtomicI64 = AtomicI64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size as i64 - layout.size() as i64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn mb(bytes: i64) -> String {
    format!("{:.1} MB", bytes as f64 / 1048576.0)
}

#[test]
#[ignore = "a measuring tool: VN_SHEET=<css file>"]
fn what_a_compiled_sheet_keeps() {
    let Ok(path) = std::env::var("VN_SHEET") else {
        eprintln!("set VN_SHEET to a stylesheet file");
        return;
    };
    let css = std::fs::read_to_string(&path).expect("VN_SHEET is readable");
    let base = LIVE.load(Ordering::Relaxed);

    let t = std::time::Instant::now();
    let first = vn_css::Engine::compile_sheets(&[css.as_str()]);
    let first_ms = t.elapsed().as_secs_f64() * 1000.0;
    let with_engine = LIVE.load(Ordering::Relaxed) - base;
    drop(first);
    let parse_only = LIVE.load(Ordering::Relaxed) - base;

    let t = std::time::Instant::now();
    let second = vn_css::Engine::compile_sheets(&[css.as_str()]);
    let second_ms = t.elapsed().as_secs_f64() * 1000.0;
    let engine = LIVE.load(Ordering::Relaxed) - base - parse_only;
    drop(second);
    let after = LIVE.load(Ordering::Relaxed) - base;

    println!("sheet: {} ({} bytes of source)", path, css.len());
    println!("first compile (parse + engine): {first_ms:.0} ms, keeps {}", mb(with_engine));
    println!("  the parse, kept by the parse cache (source, AST, UA sheet, memos): {}", mb(parse_only));
    println!("  the engine: {}", mb(with_engine - parse_only));
    println!("recompile (cache hit): {second_ms:.0} ms, engine {}", mb(engine));
    println!("after both engines are dropped: {}", mb(after));
}
