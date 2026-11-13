//! The vue-native host exe: the whole engine is the `vn_app` library beside this file (src/lib.rs);
//! this is its process-owning entry point. An application that embeds vue-native instead runs it
//! through `vn_app::embed::Builder`.

// The devtools `mem` command's Rust-heap row counts through this allocator
// (src/heapcount.rs). It belongs to the exe, not the library: a process has
// exactly one global allocator, and an embedder chooses its own.
#[cfg(vn_devtools)]
#[global_allocator]
static GLOBAL: vn_app::heapcount::Counting = vn_app::heapcount::Counting;

fn main() {
    vn_app::run_exe()
}
