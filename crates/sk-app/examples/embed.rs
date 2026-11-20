//! A program that embeds Skathi: a console app that runs the engine on a thread of its own and
//! gives the app native modules: `native:demo` with functions, tasks and an event, and
//! `native:extra` registered by a closure.
//!
//! ```text
//! cargo build -p sk-app --example embed --features prod
//! bun packages/cli/cli/sk.ts build packages/test/fixtures/embed --payload-only
//! target\x86_64-pc-windows-msvc\debug\examples\embed.exe packages\test\fixtures\embed\dist\debug\.sk\app.skpk
//! ```
//!
//! Interactive, it prints what the app reports and reads commands from stdin, the way a terminal
//! UI would drive it: `tick <n>`, `close`. With `--self-test` it runs
//! packages/test/specs/embed.spec.ts's script instead and prints one JSON line. With
//! `--sk-host-types` it prints the module's TypeScript declarations (`sk types` runs that).
//!
//! A dev build (no `prod`) takes no payload and boots the dev dist, so the app hot-reloads inside
//! the embedder.

use std::io::BufRead;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};
use sk_app::embed::{Builder, Module};
// An embedder writes `#[skathi::data]` and `#[skathi::module]`; inside Skathi the paths go
// through sk_app.
use skathi_macros::{data, module};

#[data(crate = sk_app::embed)]
struct Point {
    x: i32,
    y: i32,
}

#[data(crate = sk_app::embed)]
#[derive(Clone, serde::Serialize)]
struct Tick {
    n: u32,
}

fn thread_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned()
}

struct Demo {
    reports: mpsc::Sender<Value>,
    quiet: bool,
}

#[module(crate = sk_app::embed, events(tick = Tick))]
impl Demo {
    // Runs inside the JS call, on the Skathi thread: quick work only.
    pub fn point(&self, x: i32, y: i32) -> Point {
        Point { x, y }
    }

    pub fn refuse(&self) -> Result<u8, String> {
        Err("no such luck".into())
    }

    pub fn ui_thread(&self) -> String {
        thread_name()
    }

    // Bytes: any typed array or ArrayBuffer in, read in place; a Uint8Array out.
    pub fn checksum(&self, data: &[u8]) -> u32 {
        data.iter().map(|&b| u32::from(b)).sum()
    }

    pub fn reversed(&self, mut data: Vec<u8>) -> Vec<u8> {
        data.reverse();
        data
    }

    #[task]
    pub fn repeat(&self, data: Vec<u8>, times: usize) -> Vec<u8> {
        data.repeat(times)
    }

    // Runs on a worker; the app gets a promise.
    #[task]
    pub fn double(&self, n: u32) -> Result<u32, String> {
        std::thread::sleep(Duration::from_millis(30));
        if n == 0 {
            Err("zero has no double worth computing".into())
        } else {
            Ok(n * 2)
        }
    }

    #[task]
    pub fn worker(&self) -> String {
        thread_name()
    }

    #[task]
    pub fn crash(&self) -> u8 {
        panic!("the task fell over")
    }

    pub fn report(&self, value: Value) {
        if !self.quiet {
            println!("app reports: {value}");
        }
        let _ = self.reports.send(value);
    }
}

#[allow(deprecated)]
fn legacy(builder: Builder) -> Builder {
    builder.function("ping", |call| {
        let args: Vec<Value> = serde_json::from_str(call.args()).unwrap_or_default();
        let first = args.into_iter().next().unwrap_or(Value::Null);
        call.resolve(json!({ "pong": first }).to_string());
    })
}

fn main() {
    let mut payload = None;
    let mut self_test = false;
    for a in std::env::args().skip(1) {
        if a == "--self-test" {
            self_test = true;
        } else {
            payload = Some(a);
        }
    }

    // `report` hands what the app saw to this thread.
    let (reports_tx, reports) = mpsc::channel::<Value>();
    let mut builder = Builder::new()
        .args(["--from-embed-example"])
        // A console program keeps the keyboard; the window opens beside it.
        .activate(!self_test)
        .module(
            "demo",
            Demo {
                reports: reports_tx,
                quiet: self_test,
            },
        )
        // The closure form, for a few ad-hoc functions.
        .module("extra", |m: &mut Module| {
            m.function("sum", |a: i32, b: i32| a + b).params(["a", "b"]);
        });
    builder = legacy(builder);
    let handle_checked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if self_test {
        let checked = handle_checked.clone();
        builder = builder.setup(move |host| {
            let raw = host
                .unsafe_window_handle()
                .ok_or("no main window during setup")?;
            let handle = host.handle();
            host.on_frame(move || {
                checked.store(
                    handle.unsafe_window_handle() == Some(raw)
                        && handle.unsafe_window_handle_for(u64::MAX).is_none(),
                    std::sync::atomic::Ordering::Relaxed,
                );
            });
            Ok(())
        });
    }
    if let Some(p) = &payload {
        builder = builder.payload_file(p);
    }
    let ui = match builder.spawn() {
        Ok(ui) => ui,
        Err(e) => {
            eprintln!("embed: {e}");
            std::process::exit(2);
        }
    };

    if self_test {
        let wait = |what: &str| {
            reports
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|_| {
                    eprintln!("embed: no {what} from the app within 60 s");
                    std::process::exit(3);
                })
        };
        let first = wait("first report");
        let undeclared = ui.emit_to("demo", "tock", &1).err().map(|e| e.to_string());
        ui.emit_to("demo", "tick", &Tick { n: 1 }).expect("emit_to");
        let second = wait("tick report");
        #[allow(deprecated)]
        ui.emit("legacy", r#"{"n":2}"#).expect("emit");
        let third = wait("legacy report");
        ui.close();
        let exit = ui.wait_timeout(Duration::from_secs(15));
        let after = ui
            .emit_to("demo", "tick", &Tick { n: 3 })
            .err()
            .map(|e| e.to_string());
        let line = json!({
            "uiHandleMatched": handle_checked.load(std::sync::atomic::Ordering::Relaxed),
            "workerHasHandle": ui.unsafe_window_handle().is_some(),
            "first": first,
            "second": second,
            "third": third,
            "undeclared": undeclared,
            "exit": exit.map(|e| json!({ "clean": e.clean, "summary": e.summary })),
            "emitAfterClose": after,
        });
        println!("{line}");
        // The process is ours, and still here: Skathi's thread sleeps, and
        // returning from main ends the process the ordinary way.
        return;
    }

    println!("Skathi is running. Commands: tick <n> | close");
    let watcher = ui.clone();
    std::thread::spawn(move || {
        let exit = watcher.wait();
        println!(
            "the app closed ({}): {}",
            if exit.clean { "clean" } else { "NOT clean" },
            exit.summary
        );
        std::process::exit(0);
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some("tick"), Some(n)) => match n.parse() {
                Ok(n) => {
                    if let Err(e) = ui.emit_to("demo", "tick", &Tick { n }) {
                        println!("not sent: {e}");
                    }
                }
                Err(_) => println!("? tick <n>"),
            },
            (Some("close"), None) => ui.close(),
            (None, None) => {}
            _ => println!("? tick <n> | close"),
        }
    }
    ui.close();
    ui.wait();
}
