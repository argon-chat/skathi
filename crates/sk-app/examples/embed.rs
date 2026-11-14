//! A program that embeds Skathi: a console app that runs the engine on a thread of its own,
//! gives the app four functions and sends it events.
//!
//! ```text
//! cargo build -p sk-app --example embed --features prod
//! bun packages/cli/cli/sk.ts build packages/test/fixtures/embed --payload-only
//! target\x86_64-pc-windows-msvc\debug\examples\embed.exe packages\test\fixtures\embed\dist\.sk\app.skpk
//! ```
//!
//! Interactive, it prints every call the app makes and reads commands from
//! stdin, the way a terminal UI would drive it: `emit <event> <json>`,
//! `close`. With `--self-test` it runs packages/test/specs/embed.spec.ts's
//! script instead and prints one JSON line.
//!
//! A dev build (no `prod`) takes no payload and boots the dev dist, so the app
//! hot-reloads inside the embedder.

use std::io::BufRead;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};
use sk_app::embed::{Builder, Call};

fn args_of(call: &Call) -> Vec<Value> {
    serde_json::from_str::<Vec<Value>>(call.args()).unwrap_or_default()
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
        // Answered on the call, on the Skathi thread.
        .function("ping", |call| {
            let first = args_of(&call).first().cloned().unwrap_or(Value::Null);
            call.resolve(json!({ "pong": first }).to_string());
        })
        // Answered later, from a thread of ours: what a handler with real work
        // does, so the UI never waits on it.
        .function("add", |call| {
            let sum: f64 = args_of(&call).iter().filter_map(Value::as_f64).sum();
            let reply = call.into_reply();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                reply.resolve(json!(sum).to_string());
            });
        })
        .function("fail", |call| call.reject("no such luck"))
        .function("report", move |call| {
            let first = args_of(&call).into_iter().next().unwrap_or(Value::Null);
            if !self_test {
                println!("app reports: {first}");
            }
            let _ = reports_tx.send(first);
            call.resolve("null");
        });
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
            reports.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| {
                eprintln!("embed: no {what} from the app within 60 s");
                std::process::exit(3);
            })
        };
        let first = wait("first report");
        ui.emit("tick", r#"{"n":1}"#).expect("emit");
        let second = wait("second report");
        let refused = ui.emit("tick", "{not json").err().map(|e| e.to_string());
        ui.close();
        let exit = ui.wait_timeout(Duration::from_secs(15));
        let after = ui.emit("tick", "1").err().map(|e| e.to_string());
        let line = json!({
            "first": first,
            "second": second,
            "refusedNotJson": refused,
            "exit": exit.map(|e| json!({ "clean": e.clean, "summary": e.summary })),
            "emitAfterClose": after,
        });
        println!("{line}");
        // The process is ours, and still here: Skathi's thread sleeps, and
        // returning from main ends the process the ordinary way.
        return;
    }

    println!("Skathi is running. Commands: emit <event> <json> | close");
    let watcher = ui.clone();
    std::thread::spawn(move || {
        let exit = watcher.wait();
        println!("the app closed ({}): {}", if exit.clean { "clean" } else { "NOT clean" }, exit.summary);
        std::process::exit(0);
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut parts = line.splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("emit"), Some(event), Some(data)) => {
                if let Err(e) = ui.emit(event, data) {
                    println!("not sent: {e}");
                }
            }
            (Some("close"), None, None) => ui.close(),
            (Some(""), None, None) => {}
            _ => println!("? emit <event> <json> | close"),
        }
    }
    ui.close();
    ui.wait();
}
