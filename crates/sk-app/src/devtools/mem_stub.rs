//! `mem` off Windows: the breakdown in `mem.rs` reads the process through
//! kernel32 and ntdll (working set, heap walk, thread stacks, the address
//! space), which have no port yet. The command answers rather than failing to
//! exist, so a tool that asks learns why the numbers are missing.

use gpui::{App, Window};

use crate::json::Json;

pub(crate) fn report(_gc: bool, _code: bool, _regions: bool, _window: &mut Window, _cx: &mut App) -> Json {
    Json::obj(vec![
        ("supported", Json::Bool(false)),
        (
            "reason",
            Json::s("the memory breakdown reads Windows process counters; it has no port to this OS yet".to_string()),
        ),
    ])
}
