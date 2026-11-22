#![windows_subsystem = "windows"]

use skathi::js;

static FIRE_COST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static HANDLE: std::sync::OnceLock<skathi::Handle> = std::sync::OnceLock::new();

#[skathi::data]
pub struct Tick {
    seq: u32,
    name: String,
    values: Vec<f64>,
}

#[skathi::data]
pub struct Point {
    x: f64,
    y: f64,
}

#[skathi::data]
pub struct Item {
    id: u32,
    name: String,
    tags: Vec<String>,
    score: Option<f64>,
}

struct Bench;

#[skathi::module(events(tick = Tick))]
impl Bench {
    pub fn fire(&self, n: u32) {
        std::thread::spawn(move || {
            let handle = HANDLE.get().unwrap();
            let start = std::time::Instant::now();
            for seq in 0..n {
                let tick = Tick {
                    seq,
                    name: "progress".to_owned(),
                    values: vec![0.5, 1.5, 2.5, 3.5],
                };
                handle.emit_to("bench", "tick", &tick).unwrap();
            }
            let us = start.elapsed().as_secs_f64() * 1e6 / n.max(1) as f64;
            FIRE_COST.store(us.to_bits(), std::sync::atomic::Ordering::Relaxed);
        });
    }

    pub fn fire_cost(&self) -> f64 {
        f64::from_bits(FIRE_COST.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn flip(&self, p: Point) -> Point {
        Point { x: p.y, y: p.x }
    }

    pub fn items(&self, n: u32) -> Vec<Item> {
        (0..n)
            .map(|id| Item {
                id,
                name: format!("item {id}"),
                tags: vec!["a".into(), "b".into()],
                score: Some(id as f64 / 2.0),
            })
            .collect()
    }

    pub fn count(&self, items: Vec<Item>) -> usize {
        items.len()
    }

    pub fn add(&self, a: f64, b: f64) -> f64 {
        a + b
    }

    pub fn addi(&self, a: i32, b: i32) -> i32 {
        a.wrapping_add(b)
    }

    pub fn strlen(&self, s: String) -> usize {
        s.len()
    }

    pub fn word(&self) -> String {
        "abcdefghijklmnopqrstuvwxyz012345".to_owned()
    }

    pub fn len(&self, data: String) -> usize {
        data.len()
    }

    pub fn bytes_out(&self, n: usize) -> Vec<u8> {
        vec![7u8; n]
    }

    #[task]
    pub fn bytes_in_task(&self, data: Vec<u8>) -> usize {
        data.len()
    }

    #[task]
    pub fn bytes_out_task(&self, n: usize) -> Vec<u8> {
        vec![7u8; n]
    }

    pub fn direct(&self) -> bool {
        cfg!(feature = "direct")
    }
}

struct Bytes;

#[cfg(not(feature = "direct"))]
#[skathi::module]
impl Bytes {
    pub fn bytes_in(&self, data: Vec<u8>) -> usize {
        data.len()
    }
}

#[cfg(feature = "direct")]
#[skathi::module]
impl Bytes {
    pub fn bytes_in(&self, data: &[u8]) -> usize {
        data.len()
    }
}

fn main() {
    let payload = std::env::args().nth(1).expect("payload path");
    let ui = skathi::Builder::new()
        .payload_file(payload)
        .activate(false)
        .module("bench", Bench)
        .module("bytes", Bytes)
        .setup(|host| {
            let cx = host.js();
            let raw = cx.object();
            let add = js::function(cx, "add", 2, |cx, args| {
                Ok(js::Value::number(
                    cx.to_number(args.arg(0))? + cx.to_number(args.arg(1))?,
                ))
            })
            .map_err(|e| e.to_string())?;
            cx.set(raw, &cx.atom("add"), add)
                .map_err(|e| e.to_string())?;
            let len = js::function(cx, "len", 1, |cx, args| {
                let info = cx.typedarray_info(args.arg(0))?;
                Ok(js::Value::number(info.byte_length as f64))
            })
            .map_err(|e| e.to_string())?;
            cx.set(raw, &cx.atom("len"), len)
                .map_err(|e| e.to_string())?;
            host.expose("benchRaw", raw).map_err(|e| e.to_string())?;
            Ok(())
        })
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
    HANDLE.set(ui.clone()).ok();
    std::process::exit(if ui.wait().clean { 0 } else { 1 });
}
