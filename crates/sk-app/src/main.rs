//! The Skathi host exe: the whole engine is the `sk_app` library beside this file (src/lib.rs);
//! this is its process-owning entry point. An application that embeds Skathi instead runs it
//! through `sk_app::embed::Builder`.

// The devtools `mem` command's Rust-heap row counts through this allocator
// (src/heapcount.rs). It belongs to the exe, not the library: a process has
// exactly one global allocator, and an embedder chooses its own.
#[cfg(sk_devtools)]
#[global_allocator]
static GLOBAL: sk_app::heapcount::Counting = sk_app::heapcount::Counting;

fn main() {
    sk_app::run_exe()
}
