#![windows_subsystem = "windows"]

fn main() {
    let ui = skathi::Builder::new()
        .payload_static(include_bytes!(env!("COMPARE_PAYLOAD")))
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
    std::process::exit(if ui.wait().clean { 0 } else { 1 });
}
