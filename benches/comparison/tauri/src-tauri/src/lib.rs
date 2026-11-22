#[tauri::command]
fn add(a: f64, b: f64) -> f64 {
    a + b
}

#[tauri::command]
fn len(request: tauri::ipc::Request) -> usize {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.len(),
        tauri::ipc::InvokeBody::Json(_) => 0,
    }
}

#[tauri::command]
fn bench() -> bool {
    std::env::var_os("COMPARE_BENCH").is_some()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![add, len, bench])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
