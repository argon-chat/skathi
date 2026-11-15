//! Real Rust allocation/free capture over the SDK allocator's bounded event
//! buffer. Analysis is outside the hook and never represents JS heap/RSS as
//! individual allocation events.

use crate::{
    heapcount::{self, recording},
    json::Json,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

#[derive(Clone, Default)]
struct Counters {
    allocated_bytes: u64,
    freed_bytes: u64,
    allocations: u64,
    frees: u64,
    live_bytes: u64,
    live_blocks: u64,
}
struct Live {
    size: usize,
    site: u32,
}
#[derive(Default)]
struct Analysis {
    session: u64,
    cursor: usize,
    live: HashMap<usize, Live>,
    rows: BTreeMap<(u32, usize), Counters>,
    baseline: Option<(i64, i64, i64, u64)>,
    anomalies: u64,
}
static ANALYSIS: Mutex<Option<Analysis>> = Mutex::new(None);

impl Analysis {
    fn allocate(&mut self, address: usize, size: usize, site: u32) {
        if let Some(previous) = self.live.remove(&address) {
            self.anomalies += 1;
            let row = self.rows.entry((previous.site, previous.size)).or_default();
            row.live_bytes = row.live_bytes.saturating_sub(previous.size as u64);
            row.live_blocks = row.live_blocks.saturating_sub(1);
        }
        self.live.insert(address, Live { size, site });
        let row = self.rows.entry((site, size)).or_default();
        row.allocated_bytes += size as u64;
        row.allocations += 1;
        row.live_bytes += size as u64;
        row.live_blocks += 1;
    }
    fn free(&mut self, address: usize, size: usize) {
        let (site, recorded_size) = self
            .live
            .remove(&address)
            .map(|allocation| (allocation.site, allocation.size))
            .unwrap_or((u32::MAX, size));
        let row = self.rows.entry((site, recorded_size)).or_default();
        row.freed_bytes += recorded_size as u64;
        row.frees += 1;
        if site != u32::MAX {
            row.live_bytes = row.live_bytes.saturating_sub(recorded_size as u64);
            row.live_blocks = row.live_blocks.saturating_sub(1);
        }
    }
    fn drain(&mut self, status: &recording::Status) {
        for event in recording::drain(&mut self.cursor, recording::MAX_EVENTS) {
            match event.kind {
                1 => self.allocate(event.address, event.size, event.site),
                2 => self.free(event.address, event.size),
                3 => {
                    if recording::selected(event.address, status.sample_every) {
                        self.free(event.address, event.size);
                    }
                    if recording::selected(event.new_address, status.sample_every) {
                        self.allocate(event.new_address, event.new_size, event.site);
                    }
                }
                _ => {}
            }
        }
    }
}

fn merge(destination: &mut Counters, source: &Counters) {
    destination.allocated_bytes += source.allocated_bytes;
    destination.freed_bytes += source.freed_bytes;
    destination.allocations += source.allocations;
    destination.frees += source.frees;
    destination.live_bytes += source.live_bytes;
    destination.live_blocks += source.live_blocks;
}
fn counts(counters: &Counters) -> Vec<(&'static str, Json)> {
    vec![
        ("allocatedBytes", Json::n(counters.allocated_bytes as f64)),
        ("freedBytes", Json::n(counters.freed_bytes as f64)),
        ("allocations", Json::n(counters.allocations as f64)),
        ("frees", Json::n(counters.frees as f64)),
        ("liveBytes", Json::n(counters.live_bytes as f64)),
        ("liveBlocks", Json::n(counters.live_blocks as f64)),
        (
            "netBytes",
            Json::n(counters.allocated_bytes as f64 - counters.freed_bytes as f64),
        ),
    ]
}
fn number(
    request: &Json,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    match request.get(key).and_then(Json::as_f64) {
        None => Ok(default),
        Some(value)
            if value.is_finite()
                && value.fract() == 0.0
                && value >= min as f64
                && value <= max as f64 =>
        {
            Ok(value as usize)
        }
        Some(_) => Err(format!("{key} must be an integer between {min} and {max}")),
    }
}

/// Serialize disable with admitted commands; stale requests cannot re-arm it.
pub(super) fn cancel_recording() {
    let _suppressed = recording::suppress();
    let _analysis = ANALYSIS.lock().unwrap_or_else(|error| error.into_inner());
    recording::stop();
}

/// Does not touch JSC or GPUI; safe on the authenticated listener thread.
pub(super) fn command_authenticated(request: &Json, generation: u64) -> Result<Json, String> {
    let _suppressed = recording::suppress();
    let operation = request.get("op").and_then(Json::as_str).unwrap_or("status");
    let mut guard = ANALYSIS.lock().unwrap_or_else(|error| error.into_inner());
    if !super::enabled()
        || generation != super::ACCEPT_GEN.load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(
            "The devtools authorization was revoked before this allocation command could run"
                .into(),
        );
    }
    if operation == "start" {
        let capacity = number(
            request,
            "maxEvents",
            recording::MAX_EVENTS,
            1024,
            recording::MAX_EVENTS,
        )?;
        let sample_every = number(request, "sampleEvery", 1, 1, 4096)?;
        let status = recording::start(capacity, sample_every);
        *guard = Some(Analysis {
            session: status.session,
            baseline: heapcount::snapshot(),
            ..Analysis::default()
        });
    } else if operation == "stop" {
        recording::stop();
    } else if operation == "clear" {
        recording::clear();
        *guard = None;
    } else if !matches!(operation, "status" | "snapshot") {
        return Err("allocations op must be start, stop, status, snapshot, or clear".into());
    }
    let status = recording::status();
    let analysis = guard.get_or_insert_with(|| Analysis {
        session: status.session,
        baseline: heapcount::snapshot(),
        ..Analysis::default()
    });
    if analysis.session != status.session {
        *analysis = Analysis {
            session: status.session,
            baseline: heapcount::snapshot(),
            ..Analysis::default()
        };
    }
    analysis.drain(&status);
    let sites = recording::sites();
    let minimum = number(request, "minSize", 0, 0, usize::MAX)?;
    let maximum = number(request, "maxSize", usize::MAX, 0, usize::MAX)?;
    if minimum > maximum {
        return Err("minSize exceeds maxSize".into());
    }
    let limit = number(request, "limit", 300, 1, 2000)?;
    let category_filter = request
        .get("category")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_lowercase();
    let site_filter = request
        .get("site")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_lowercase();
    let mut grouped: BTreeMap<(u32, usize), Counters> = BTreeMap::new();
    let mut totals = Counters::default();
    for (&(site, size), counters) in &analysis.rows {
        merge(&mut totals, counters);
        if size < minimum || size > maximum {
            continue;
        }
        let class = size.checked_next_power_of_two().unwrap_or(usize::MAX);
        merge(grouped.entry((site, class)).or_default(), counters);
    }
    let mut rows: Vec<_> = grouped
        .into_iter()
        .filter_map(|((site, class), counters)| {
            let (category, label) = match site {
                0 => ("unscoped", "No explicit allocation scope"),
                u32::MAX => ("preexisting", "Freed allocation created before capture"),
                _ => sites
                    .get(site as usize - 1)
                    .map(|site| (site.category.as_str(), site.name.as_str()))
                    .unwrap_or(("unscoped", "Scope registry limit reached")),
            };
            if !category.to_lowercase().contains(&category_filter)
                || !label.to_lowercase().contains(&site_filter)
            {
                return None;
            }
            Some((category.to_owned(), label.to_owned(), class, counters))
        })
        .collect();
    rows.sort_by_key(|(_, _, _, counters)| {
        (
            std::cmp::Reverse(counters.live_bytes),
            std::cmp::Reverse(counters.allocated_bytes),
        )
    });
    let matched_rows = rows.len();
    rows.truncate(limit);
    let rows = rows
        .into_iter()
        .map(|(category, site, class, counters)| {
            let mut fields = counts(&counters);
            fields.extend([
                ("category", Json::s(category)),
                ("site", Json::s(site)),
                ("sizeClass", Json::n(class as f64)),
            ]);
            Json::obj(fields)
        })
        .collect();
    let heap = match (analysis.baseline, heapcount::snapshot()) {
        (Some(before), Some(now)) => Json::obj(vec![
            ("liveBytes", Json::n(now.0 as f64)),
            ("liveBlocks", Json::n(now.1 as f64)),
            ("deltaBytes", Json::n((now.0 - before.0) as f64)),
            ("deltaBlocks", Json::n((now.1 - before.1) as f64)),
            (
                "allocationsSinceStart",
                Json::n(now.3.saturating_sub(before.3) as f64),
            ),
        ]),
        _ => Json::Null,
    };
    Ok(Json::obj(vec![
        ("available", Json::Bool(true)),
        ("session", Json::n(status.session as f64)),
        ("recording", Json::Bool(status.recording)),
        ("startedAtMs", Json::n(status.started_at_ms)),
        ("elapsedMs", Json::n(status.elapsed_ms)),
        ("sampleEvery", Json::n(status.sample_every as f64)),
        ("maxEvents", Json::n(status.capacity as f64)),
        ("capturedEvents", Json::n(analysis.cursor as f64)),
        ("droppedEvents", Json::n(status.dropped as f64)),
        (
            "complete",
            Json::Bool(
                status.dropped == 0
                    && analysis.anomalies == 0
                    && matched_rows <= limit
                    && analysis.cursor >= status.reserved.min(status.capacity),
            ),
        ),
        (
            "coverage",
            Json::obj(vec![
                (
                    "rust",
                    Json::s(
                        "Global allocator events; pointer-hash sampling, observed bytes are not extrapolated",
                    ),
                ),
                (
                    "js",
                    Json::s("Not recorded: JS object allocation hooks are unavailable"),
                ),
                (
                    "jsc",
                    Json::s(
                        "Not recorded: JSC/mimalloc allocations are outside the Rust allocator",
                    ),
                ),
                (
                    "native",
                    Json::s("Not recorded: COM, GPU driver, OS and C/C++ allocators"),
                ),
                (
                    "sites",
                    Json::s(
                        "Explicit native scopes with source locations; unscoped events remain unscoped. No inferred stacks.",
                    ),
                ),
                (
                    "allocatorObserved",
                    Json::Bool(status.reserved != 0 || heapcount::on()),
                ),
                ("bufferBytes", Json::n(recording::reserved_bytes() as f64)),
            ]),
        ),
        ("totals", Json::obj(counts(&totals))),
        ("heap", heap),
        ("rows", Json::arr(rows)),
        ("matchedRows", Json::n(matched_rows as f64)),
        ("truncated", Json::Bool(matched_rows > limit)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_rows_track_original_site_and_preexisting_frees_separately() {
        let mut analysis = Analysis::default();
        analysis.allocate(0x1000, 80, 3);
        analysis.allocate(0x2000, 160, 7);
        analysis.free(0x1000, 80);
        analysis.free(0x3000, 40);
        assert_eq!(analysis.rows[&(3, 80)].live_bytes, 0);
        assert_eq!(analysis.rows[&(3, 80)].freed_bytes, 80);
        assert_eq!(analysis.rows[&(7, 160)].live_bytes, 160);
        assert_eq!(analysis.rows[&(u32::MAX, 40)].freed_bytes, 40);
        assert_eq!(analysis.live.len(), 1);
    }
    #[test]
    fn stale_authorization_cannot_start_capture() {
        let request = Json::obj(vec![("op", Json::s("start"))]);
        let stale = super::super::ACCEPT_GEN
            .load(std::sync::atomic::Ordering::SeqCst)
            .wrapping_add(1);
        assert!(command_authenticated(&request, stale).is_err());
    }
}

/// Stops a disconnected inspector's capture while preserving its report.
pub(super) fn stop_recording() {
    let _guard = ANALYSIS.lock().unwrap_or_else(|error| error.into_inner());
    recording::stop();
}
