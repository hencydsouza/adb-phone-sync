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

/// Migrations registered against `DB_URL`, kept in sync with the Drizzle
/// migration files under `src/db/migrations/`.
///
/// IMPORTANT: every `.sql` file Drizzle generates there (tracked in
/// `src/db/migrations/meta/_journal.json`) MUST have a corresponding
/// `Migration` entry below, in the same order, or the schema change never
/// reaches the real app database (see commit 862d3e3). The
/// `migrations_registered_match_journal` test in this file fails loudly if
/// the counts drift apart -- run `cargo test` after adding a migration.
fn migrations() -> Vec<tauri_plugin_sql::Migration> {
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
    ]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(DB_URL, migrations())
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

#[cfg(test)]
mod tests {
    use super::migrations;

    /// Drift guard for the exact bug fixed in commit 862d3e3: Drizzle's
    /// `_journal.json` is the source of truth for how many migration files
    /// exist, and `migrations()` must register exactly that many
    /// `Migration` entries against `DB_URL`. If a future task runs
    /// `drizzle-kit generate` and produces a new `.sql` file, this test
    /// fails until a matching entry is hand-added to `migrations()`.
    #[test]
    fn migrations_registered_match_journal() {
        let journal = include_str!("../../src/db/migrations/meta/_journal.json");
        let journal: serde_json::Value =
            serde_json::from_str(journal).expect("_journal.json must be valid JSON");
        let journal_entry_count = journal
            .get("entries")
            .and_then(|entries| entries.as_array())
            .expect("_journal.json must have an `entries` array")
            .len();

        let registered_count = migrations().len();

        assert_eq!(
            registered_count, journal_entry_count,
            "migrations() registers {registered_count} Migration entr{registered_suffix}, but \
             src/db/migrations/meta/_journal.json lists {journal_entry_count} migration \
             file{journal_suffix}. Every Drizzle migration file needs a matching \
             tauri_plugin_sql::Migration entry in migrations() (src-tauri/src/lib.rs), in \
             generation order, or the schema change silently never reaches the app database.",
            registered_suffix = if registered_count == 1 { "y" } else { "ies" },
            journal_suffix = if journal_entry_count == 1 { "" } else { "s" },
        );
    }
}
