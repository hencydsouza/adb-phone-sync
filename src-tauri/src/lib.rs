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

/// Connection string used for the app's SQLite database. Must match the
/// string passed to `Database.load(...)` in `src/db/client.ts` exactly,
/// otherwise the migrations registered here never apply to the database
/// the frontend actually opens.
const DB_URL: &str = "sqlite:adb-phone-sync.db";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(
                    DB_URL,
                    vec![
                        tauri_plugin_sql::Migration {
                            version: 1,
                            description: "initial schema",
                            sql: include_str!("../../src/db/migrations/0000_stormy_the_fallen.sql"),
                            kind: tauri_plugin_sql::MigrationKind::Up,
                        },
                        tauri_plugin_sql::Migration {
                            version: 2,
                            description: "add destination_path to devices",
                            sql: include_str!("../../src/db/migrations/0001_oval_magneto.sql"),
                            kind: tauri_plugin_sql::MigrationKind::Up,
                        },
                    ],
                )
                .build(),
        )
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
