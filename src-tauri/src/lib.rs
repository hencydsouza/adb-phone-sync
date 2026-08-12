// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod classify;
mod device_scan;
mod devices;
mod space;
mod sync;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_sql::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            greet,
            devices::list_devices,
            device_scan::classify_suggest,
            space::space_check,
            sync::orchestration::run_backup,
            sync::orchestration::run_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
