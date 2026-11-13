use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
use std::rc::Rc;
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
    println!("  what outlives the engine (UA sheet, memos; the parse cache, while there was one): {}", mb(parse_only));
    println!("  the engine: {}", mb(with_engine - parse_only));
    println!("recompile of the same text: {second_ms:.0} ms, engine {}", mb(engine));
    println!("after both engines are dropped: {}", mb(after));
}

#[test]
#[ignore = "a measuring tool: VN_SHEET=<css file>"]
fn where_the_engine_goes() {
    let Ok(path) = std::env::var("VN_SHEET") else {
        eprintln!("set VN_SHEET to a stylesheet file");
        return;
    };
    let css = std::fs::read_to_string(&path).expect("VN_SHEET is readable");
    // Warm the per-thread memos (UA sheet, reparsed selectors) so they are not counted as the
    // engine's.
    drop(vn_css::Engine::compile_sheets(&[css.as_str()]));
    let base = LIVE.load(Ordering::Relaxed);
    let engine = vn_css::Engine::compile_sheets(&[css.as_str()]);
    let total = LIVE.load(Ordering::Relaxed) - base;
    let parts = engine.into_parts_for_measuring();
    println!("engine: {} for {} bytes of source", mb(total), css.len());
    println!("{:<70} {:>9} {:>9}", "part", "inline", "heap");
    let mut accounted = 0;
    for (name, inline, value) in parts {
        // How many of the refinements are the same value: what interning them would leave.
        type Pair = (Option<Rc<gpui::StyleRefinement>>, Option<Rc<gpui::StyleRefinement>>);
        if let Some(pairs) = value.downcast_ref::<Vec<Pair>>() {
            let all: Vec<&Rc<gpui::StyleRefinement>> =
                pairs.iter().flat_map(|(a, b)| [a.as_ref(), b.as_ref()]).flatten().collect();
            let allocations: HashSet<*const gpui::StyleRefinement> = all.iter().map(|r| Rc::as_ptr(r)).collect();
            let values: HashSet<String> = all.iter().map(|r| format!("{:?}", ***r)).collect();
            println!(
                "  ({} refinements of {} bytes each: {} allocations, {} distinct values)",
                all.len(),
                std::mem::size_of::<gpui::StyleRefinement>(),
                allocations.len(),
                values.len()
            );
        }
        let before = LIVE.load(Ordering::Relaxed);
        let unit = value.downcast_ref::<()>().is_some();
        drop(value);
        let freed = before - LIVE.load(Ordering::Relaxed);
        let inline = inline as i64;
        if unit {
            println!("{name:<70} {:>9} {:>9}", mb(inline), "");
            accounted += inline;
        } else {
            println!("{name:<70} {:>9} {:>9}", if inline > 0 { mb(inline) } else { String::new() }, mb(freed - inline));
            accounted += freed - inline;
        }
    }
    println!("accounted: {} of {}", mb(accounted), mb(total));
}
