# Android Backup/Restore Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the backup/restore feature described in
`docs/plans/2026-08-12-android-backup-restore-sync-design.md` — a Tauri app
that backs up and restores an Android phone's shared storage to/from a PC,
using a bundled, frozen `adbsync` binary driven by Rust, with SQLite-backed
run history and a React/astryxdesign UI.

**Architecture:** Rust backend shells out to bundled `adbsync.exe` (frozen
from the vendored `better-adb-sync` submodule) and `adb.exe`, one subprocess
call per included folder, never a whole-tree call. Rust owns all path
construction (trailing-slash safe) and writes run history to SQLite. React
frontend (astryxdesign) drives device selection, folder classification
review, and progress, calling Tauri commands.

**Tech Stack:** Tauri 2 / Rust, React 19, `@astryxdesign/core`, SQLite via
`@tauri-apps/plugin-sql` + `drizzle-orm` (sqlite-proxy driver), vendored
`better-adb-sync` (Python, frozen via PyInstaller), bundled Android
platform-tools.

**Read first:** `docs/research/` (all files) and
`docs/plans/2026-08-12-android-backup-restore-sync-design.md` — this plan
assumes that context and doesn't repeat the rationale, only the "what to
build."

---

### Task 1: Vendor `better-adb-sync` as a pinned submodule

**Files:**
- Create: `.gitmodules`
- Create: `third_party/better-adb-sync/` (submodule checkout)

**Step 1: Add the submodule**

Run:
```bash
git submodule add https://github.com/jb2170/better-adb-sync.git third_party/better-adb-sync
```

**Step 2: Pin to the verified release tag**

Run:
```bash
cd third_party/better-adb-sync
git checkout v1.4.0
cd ../..
```

**Step 3: Verify the pin and the package layout**

Run: `cat third_party/better-adb-sync/pyproject.toml`
Expected: `name = "BetterADBSync"`, `[project.scripts] adbsync =
"BetterADBSync:main"`, matching `docs/research/adbsync-tooling.md`.

**Step 4: Commit**

```bash
git add .gitmodules third_party/better-adb-sync
git commit -m "build: vendor better-adb-sync v1.4.0 as submodule"
```

---

### Task 2: Freeze `adbsync` into a standalone Windows executable

**Files:**
- Create: `scripts/adbsync_entry.py`
- Create: `scripts/build-adbsync.ps1`
- Modify: `.gitignore` (ignore the PyInstaller `build/`/`dist/` output dirs)

**Step 1: Write the PyInstaller entry-point wrapper**

`better-adb-sync` only exposes a `console_scripts` entry point
(`BetterADBSync:main`), not a standalone script — PyInstaller needs a real
`.py` file to target.

```python
# scripts/adbsync_entry.py
"""PyInstaller entry point for the vendored better-adb-sync package."""
import sys
sys.path.insert(0, "third_party/better-adb-sync/src")

from BetterADBSync import main

if __name__ == "__main__":
    main()
```

**Step 2: Write the build script**

```powershell
# scripts/build-adbsync.ps1
$ErrorActionPreference = "Stop"

pip install pyinstaller

pyinstaller `
  --onefile `
  --name adbsync `
  --paths third_party/better-adb-sync/src `
  scripts/adbsync_entry.py

$target = "src-tauri/binaries/adbsync-x86_64-pc-windows-msvc.exe"
New-Item -ItemType Directory -Force -Path "src-tauri/binaries" | Out-Null
Copy-Item -Force "dist/adbsync.exe" $target

Write-Host "Built $target"
```

**Step 3: Run the build**

Run: `powershell -File scripts/build-adbsync.ps1`
Expected: `Built src-tauri/binaries/adbsync-x86_64-pc-windows-msvc.exe`, file
exists.

**Step 4: Smoke-test the frozen binary**

Run: `& "src-tauri/binaries/adbsync-x86_64-pc-windows-msvc.exe" --version`
Expected: prints `1.4.0` (matches `__version__` in the vendored source), exit
code 0. If this fails, the freeze is broken — do not proceed to Task 3 until
it passes.

**Step 5: Commit**

```bash
git add scripts/adbsync_entry.py scripts/build-adbsync.ps1 .gitignore
git commit -m "build: add PyInstaller freeze script for adbsync"
```

(The frozen `.exe` itself is a build artifact — do not commit it; Task 3
wires it into the Tauri bundle via `externalBin`, generated at build time.)

---

### Task 3: Bundle platform-tools and the frozen adbsync as Tauri sidecars

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Create: `src-tauri/binaries/` (adb.exe, adbsync.exe — target-triple suffixed)
- Create: `scripts/fetch-platform-tools.ps1`

**Step 1: Write a script to fetch and place platform-tools**

```powershell
# scripts/fetch-platform-tools.ps1
$ErrorActionPreference = "Stop"
$zipPath = "$env:USERPROFILE\Downloads\platform-tools-latest-windows.zip"
$dest = "src-tauri/binaries"
New-Item -ItemType Directory -Force -Path $dest | Out-Null

Expand-Archive -Path $zipPath -DestinationPath "$env:TEMP\platform-tools-extract" -Force
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\adb.exe" "$dest\adb-x86_64-pc-windows-msvc.exe"
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\AdbWinApi.dll" "$dest\AdbWinApi.dll"
Copy-Item "$env:TEMP\platform-tools-extract\platform-tools\AdbWinUsbApi.dll" "$dest\AdbWinUsbApi.dll"

Write-Host "Placed adb.exe + DLLs in $dest"
```

Run it: `powershell -File scripts/fetch-platform-tools.ps1`
Expected: `src-tauri/binaries/adb-x86_64-pc-windows-msvc.exe`,
`AdbWinApi.dll`, `AdbWinUsbApi.dll` all present.

**Step 2: Register both binaries as Tauri external binaries, and the DLLs as resources**

Edit `src-tauri/tauri.conf.json`, adding to the top-level object:

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "externalBin": ["binaries/adb", "binaries/adbsync"],
    "resources": ["binaries/AdbWinApi.dll", "binaries/AdbWinUsbApi.dll"]
  }
```

(`externalBin` entries are given without the target-triple suffix — Tauri
appends it automatically when resolving `binaries/adb-x86_64-pc-windows-msvc.exe`
and `binaries/adbsync-x86_64-pc-windows-msvc.exe`.)

**Step 3: Add the shell plugin (needed to invoke sidecars) and permission**

Run: `cd src-tauri && cargo add tauri-plugin-shell && cd ..`

Edit `src-tauri/capabilities/default.json`, add `"shell:allow-execute"` to
`permissions`.

**Step 4: Verify sidecar resolution**

This can't be fully verified until Task 6 (`devices::list`) actually spawns
the sidecar — flag as a dependency. For now, verify the build picks up the
binaries without erroring:

Run: `cd src-tauri && cargo tauri build --debug 2>&1 | tail -30`
Expected: no `externalBin`/`resources` errors from Tauri's bundler.

**Step 5: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/capabilities/default.json src-tauri/Cargo.toml scripts/fetch-platform-tools.ps1
git commit -m "build: bundle adb + frozen adbsync as Tauri sidecars"
```

(`src-tauri/binaries/*` stays gitignored — each dev/CI run fetches/builds
them via the two scripts above.)

---

### Task 4: Set up SQLite storage (Tauri SQL plugin + Drizzle)

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json`
- Modify: `package.json`
- Create: `src/db/schema.ts`, `src/db/client.ts`, `drizzle.config.ts`

**Step 1: Add the Rust and JS SQL dependencies**

```bash
cd src-tauri && cargo add tauri-plugin-sql --features sqlite && cd ..
bun add @tauri-apps/plugin-sql drizzle-orm
bun add -D drizzle-kit
```

**Step 2: Register the plugin in Rust**

Edit `src-tauri/src/lib.rs`, add `.plugin(tauri_plugin_sql::Builder::default().build())`
to the `tauri::Builder::default()` chain (alongside the existing
`tauri_plugin_opener::init()`).

**Step 3: Add the SQL permission**

Edit `src-tauri/capabilities/default.json`, add `"sql:default"` to
`permissions`.

**Step 4: Define the Drizzle schema**

```typescript
// src/db/schema.ts
import { sqliteTable, text, integer } from "drizzle-orm/sqlite-core";

export const devices = sqliteTable("devices", {
  serial: text("serial").primaryKey(),
  displayName: text("display_name").notNull(),
  firstSeen: integer("first_seen", { mode: "timestamp" }).notNull(),
  lastSeen: integer("last_seen", { mode: "timestamp" }).notNull(),
});

export const folderRules = sqliteTable("folder_rules", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  deviceSerial: text("device_serial").notNull().references(() => devices.serial),
  path: text("path").notNull(),
  decision: text("decision", { enum: ["include", "skip"] }).notNull(),
  source: text("source", { enum: ["heuristic", "manual"] }).notNull(),
  updatedAt: integer("updated_at", { mode: "timestamp" }).notNull(),
});

export const runs = sqliteTable("runs", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  deviceSerial: text("device_serial").notNull().references(() => devices.serial),
  type: text("type", { enum: ["backup", "restore"] }).notNull(),
  startedAt: integer("started_at", { mode: "timestamp" }).notNull(),
  finishedAt: integer("finished_at", { mode: "timestamp" }),
  status: text("status", { enum: ["running", "completed", "failed", "cancelled"] }).notNull(),
});

export const runItems = sqliteTable("run_items", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  runId: integer("run_id").notNull().references(() => runs.id),
  path: text("path").notNull(),
  status: text("status", { enum: ["synced", "outdated", "broken", "skipped", "error"] }).notNull(),
  bytesTransferred: integer("bytes_transferred"),
  fileCount: integer("file_count"),
  errorMessage: text("error_message"),
  finishedAt: integer("finished_at", { mode: "timestamp" }),
});
```

**Step 5: Wire the Drizzle sqlite-proxy client to the Tauri SQL plugin**

```typescript
// src/db/client.ts
import { drizzle } from "drizzle-orm/sqlite-proxy";
import Database from "@tauri-apps/plugin-sql";
import * as schema from "./schema";

const sqlite = await Database.load("sqlite:adb-phone-sync.db");

export const db = drizzle(
  async (sql, params, method) => {
    if (method === "run" || method === "all") {
      const rows = await sqlite.select<Record<string, unknown>[]>(sql, params);
      return { rows: rows.map((row) => Object.values(row)) };
    }
    const result = await sqlite.execute(sql, params);
    return { rows: [], rowsAffected: result.rowsAffected } as unknown as { rows: unknown[] };
  },
  { schema },
);
```

Note for whoever implements this: `@tauri-apps/plugin-sql` and
`drizzle-orm`'s sqlite-proxy signature are both fast-moving APIs — verify the
exact `select`/`execute` method names and the proxy callback's expected
return shape against the installed package versions (`bunx tauri add sql
--help` / the installed `drizzle-orm` version's sqlite-proxy typings) before
trusting this snippet verbatim.

**Step 6: Generate and apply the initial migration**

```typescript
// drizzle.config.ts
import { defineConfig } from "drizzle-kit";

export default defineConfig({
  schema: "./src/db/schema.ts",
  dialect: "sqlite",
  out: "./src/db/migrations",
});
```

Run: `bunx drizzle-kit generate`
Expected: a new file under `src/db/migrations/` with `CREATE TABLE`
statements for all four tables.

**Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/lib.rs src-tauri/capabilities/default.json package.json bun.lock src/db drizzle.config.ts
git commit -m "feat: add SQLite storage via Tauri SQL plugin + Drizzle schema"
```

---

### Task 5: `ANDROID`/`LOCAL` path-pair builder (Rust, TDD)

This is the single highest-priority piece of logic in the plan — it's the
direct fix for the confirmed nesting bug at `BetterADBSync/__init__.py:323`.

**Files:**
- Create: `src-tauri/src/sync/path_pair.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod sync;`)

**Step 1: Write the failing test**

```rust
// src-tauri/src/sync/path_pair.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_pair_always_has_trailing_slash_on_android_source() {
        let pair = build_pull_pair("/storage/emulated/0/DCIM", r"C:\dest\DCIM");
        assert_eq!(pair.android, "/storage/emulated/0/DCIM/");
        assert_eq!(pair.local, r"C:\dest\DCIM");
    }

    #[test]
    fn push_pair_always_has_trailing_slash_on_local_source() {
        let pair = build_push_pair(r"C:\dest\DCIM", "/storage/emulated/0/DCIM");
        assert_eq!(pair.local, r"C:\dest\DCIM\");
        assert_eq!(pair.android, "/storage/emulated/0/DCIM");
    }

    #[test]
    fn does_not_double_up_an_existing_trailing_slash() {
        let pair = build_pull_pair("/storage/emulated/0/DCIM/", r"C:\dest\DCIM");
        assert_eq!(pair.android, "/storage/emulated/0/DCIM/");
    }
}
```

**Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test path_pair`
Expected: FAIL — `build_pull_pair`/`build_push_pair`/`PathPair` not found.

**Step 3: Implement**

```rust
// src-tauri/src/sync/path_pair.rs (above the #[cfg(test)] module)
#[derive(Debug, PartialEq, Eq)]
pub struct PathPair {
    pub android: String,
    pub local: String,
}

fn with_trailing_slash(path: &str, sep: char) -> String {
    if path.ends_with(sep) {
        path.to_string()
    } else {
        format!("{path}{sep}")
    }
}

/// Build the ANDROID/LOCAL pair for `adbsync pull`.
/// The ANDROID (source) side always gets a trailing slash, so its *contents*
/// land in `local_dest` rather than nesting the source folder inside it.
pub fn build_pull_pair(android_source: &str, local_dest: &str) -> PathPair {
    PathPair {
        android: with_trailing_slash(android_source, '/'),
        local: local_dest.to_string(),
    }
}

/// Build the LOCAL/ANDROID pair for `adbsync push`.
/// The LOCAL (source) side always gets a trailing slash, mirroring pull.
pub fn build_push_pair(local_source: &str, android_dest: &str) -> PathPair {
    PathPair {
        local: with_trailing_slash(local_source, '\\'),
        android: android_dest.to_string(),
    }
}
```

**Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test path_pair`
Expected: `3 passed`.

**Step 5: Commit**

```bash
git add src-tauri/src/sync/path_pair.rs src-tauri/src/lib.rs
git commit -m "feat: add trailing-slash-safe ANDROID/LOCAL path pair builder"
```

---

### Task 6: `devices::list` Tauri command

**Files:**
- Create: `src-tauri/src/devices.rs`
- Modify: `src-tauri/src/lib.rs`

**Step 1: Write the failing test for output parsing**

```rust
// src-tauri/src/devices.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adb_devices_dash_l_output() {
        let raw = "List of devices attached\n\
                    00070344C000047        device usb:1-1 product:Nothing model:Phone_2a device:Spacewar transport_id:3\n\
                    emulator-5554           offline\n";
        let devices = parse_adb_devices_output(raw);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].serial, "00070344C000047");
        assert_eq!(devices[0].state, "device");
        assert_eq!(devices[1].serial, "emulator-5554");
        assert_eq!(devices[1].state, "offline");
    }

    #[test]
    fn ignores_the_header_and_blank_lines() {
        let devices = parse_adb_devices_output("List of devices attached\n\n");
        assert!(devices.is_empty());
    }
}
```

**Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test devices::`
Expected: FAIL — `parse_adb_devices_output` not found.

**Step 3: Implement the parser and the Tauri command**

```rust
// src-tauri/src/devices.rs (above the #[cfg(test)] module)
use tauri_plugin_shell::ShellExt;

#[derive(serde::Serialize, Debug, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    pub state: String,
}

pub fn parse_adb_devices_output(raw: &str) -> Vec<Device> {
    raw.lines()
        .skip(1) // "List of devices attached"
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            Some(Device { serial: serial.to_string(), state: state.to_string() })
        })
        .collect()
}

#[tauri::command]
pub async fn list_devices(app: tauri::AppHandle) -> Result<Vec<Device>, String> {
    let (mut rx, _child) = app
        .shell()
        .sidecar("adb")
        .map_err(|e| e.to_string())?
        .args(["devices", "-l"])
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut output = String::new();
    while let Some(event) = rx.recv().await {
        if let tauri_plugin_shell::process::CommandEvent::Stdout(bytes) = event {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    Ok(parse_adb_devices_output(&output))
}
```

Register in `src-tauri/src/lib.rs`:
```rust
mod devices;
// ...
.invoke_handler(tauri::generate_handler![greet, devices::list_devices])
```

**Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test devices::`
Expected: `2 passed`.

**Step 5: Manual verification against the real bundled sidecar**

This needs a connected device and the Task 3 sidecar bundling done — cannot
be unit tested. Run the dev app (`bun run tauri dev`), call `list_devices`
from the devtools console, confirm the real connected device serial appears.
Add to the manual QA checklist (Task 16).

**Step 6: Commit**

```bash
git add src-tauri/src/devices.rs src-tauri/src/lib.rs
git commit -m "feat: add devices::list_devices Tauri command"
```

---

### Task 7: Folder classification heuristic (Rust, TDD)

**Files:**
- Create: `src-tauri/src/classify.rs`
- Modify: `src-tauri/src/lib.rs`

**Step 1: Write the failing test, using the real test-device table as fixture data**

```rust
// src-tauri/src/classify.rs
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, is_empty: bool) -> FolderEntry {
        FolderEntry { name: name.to_string(), is_dir, is_empty }
    }

    #[test]
    fn personal_media_folders_are_included() {
        for name in ["DCIM", "Pictures", "Movies", "Music", "Documents", "Download"] {
            let suggestion = classify(&entry(name, true, false), &[]);
            assert_eq!(suggestion, Decision::Include, "{name} should be included");
        }
    }

    #[test]
    fn stock_noise_folders_are_skipped() {
        for name in ["Alarms", "Notifications", "Audiobooks", "Podcasts", "Recordings"] {
            let suggestion = classify(&entry(name, true, true), &[]);
            assert_eq!(suggestion, Decision::Skip, "{name} should be skipped");
        }
    }

    #[test]
    fn android_data_and_obb_are_always_excluded() {
        assert_eq!(classify(&entry("Android/data", true, false), &[]), Decision::Skip);
        assert_eq!(classify(&entry("Android/obb", true, false), &[]), Decision::Skip);
    }

    #[test]
    fn root_folder_shadowed_by_newer_android_media_copy_is_flagged_stale() {
        // root /WhatsApp (older) vs Android/media/com.whatsapp/WhatsApp (newer)
        let siblings = [entry("Android/media/com.whatsapp/WhatsApp", true, false)];
        let suggestion = classify_with_mtime_hint(&entry("WhatsApp", true, false), &siblings, false /* not newer */);
        assert_eq!(suggestion, Decision::SkipStaleDuplicate);
    }
}
```

**Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test classify::`
Expected: FAIL — types/functions not found.

**Step 3: Implement**

```rust
// src-tauri/src/classify.rs (above the #[cfg(test)] module)
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub enum Decision {
    Include,
    Skip,
    SkipStaleDuplicate,
}

#[derive(Debug)]
pub struct FolderEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_empty: bool,
}

const PERSONAL_MEDIA: &[&str] = &["DCIM", "Pictures", "Movies", "Music", "Documents", "Download"];
const STOCK_NOISE: &[&str] = &["Alarms", "Notifications", "Audiobooks", "Podcasts", "Recordings"];
const ALWAYS_EXCLUDED_PREFIXES: &[&str] = &["Android/data", "Android/obb"];

pub fn classify(entry: &FolderEntry, siblings: &[FolderEntry]) -> Decision {
    classify_with_mtime_hint(entry, siblings, true)
}

pub fn classify_with_mtime_hint(entry: &FolderEntry, siblings: &[FolderEntry], is_newer_than_shadow: bool) -> Decision {
    if ALWAYS_EXCLUDED_PREFIXES.iter().any(|p| entry.name.starts_with(p)) {
        return Decision::Skip;
    }
    if PERSONAL_MEDIA.contains(&entry.name.as_str()) {
        return Decision::Include;
    }
    if STOCK_NOISE.contains(&entry.name.as_str()) || entry.is_empty {
        return Decision::Skip;
    }
    // stale-duplicate check: a top-level folder name that also appears as
    // `Android/media/<pkg>/<name>` is suspect — prefer whichever is newer.
    let shadow_suffix = format!("/{}", entry.name);
    let has_media_shadow = siblings.iter().any(|s| s.name.ends_with(&shadow_suffix) && s.name.starts_with("Android/media/"));
    if has_media_shadow && !is_newer_than_shadow {
        return Decision::SkipStaleDuplicate;
    }
    Decision::Include // default: unknown folder, human should review
}
```

Register `mod classify;` in `src-tauri/src/lib.rs`.

**Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test classify::`
Expected: `4 passed`.

**Step 5: Commit**

```bash
git add src-tauri/src/classify.rs src-tauri/src/lib.rs
git commit -m "feat: add folder classification heuristic"
```

---

### Task 8: Free-space and cloud-sync-path preflight checks (Rust, TDD)

**Files:**
- Create: `src-tauri/src/space.rs`
- Modify: `src-tauri/src/lib.rs`

**Step 1: Write the failing tests**

```rust
// src-tauri/src/space.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_when_free_space_is_below_estimate() {
        let result = check_space(estimated_bytes: 10_000, free_bytes: 5_000);
        assert!(!result.has_enough_space);
    }

    #[test]
    fn passes_when_free_space_exceeds_estimate() {
        let result = check_space(estimated_bytes: 5_000, free_bytes: 10_000);
        assert!(result.has_enough_space);
    }

    #[test]
    fn detects_onedrive_paths() {
        assert!(is_cloud_synced_path(r"C:\Users\hency\OneDrive\Desktop\nothing2a"));
        assert!(!is_cloud_synced_path(r"D:\Backups\phone"));
    }

    #[test]
    fn detects_dropbox_paths() {
        assert!(is_cloud_synced_path(r"C:\Users\hency\Dropbox\phone"));
    }
}
```

**Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test space::`
Expected: FAIL — functions not found.

**Step 3: Implement**

```rust
// src-tauri/src/space.rs (above the #[cfg(test)] module)
pub struct SpaceCheck {
    pub has_enough_space: bool,
    pub free_bytes: u64,
    pub estimated_bytes: u64,
}

pub fn check_space(estimated_bytes: u64, free_bytes: u64) -> SpaceCheck {
    SpaceCheck {
        has_enough_space: free_bytes > estimated_bytes,
        free_bytes,
        estimated_bytes,
    }
}

const CLOUD_SYNC_MARKERS: &[&str] = &["OneDrive", "Dropbox", "Google Drive", "iCloudDrive"];

pub fn is_cloud_synced_path(path: &str) -> bool {
    CLOUD_SYNC_MARKERS.iter().any(|marker| path.contains(marker))
}
```

Register `mod space;` in `src-tauri/src/lib.rs`. (Actual disk free-space
lookup — e.g. via the `fs4` or `sysinfo` crate — and the `adb shell du`
size estimate are wired in as part of Task 9's Tauri command, since they
need real I/O; this task is the pure decision logic only.)

**Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test space::`
Expected: `4 passed`.

**Step 5: Commit**

```bash
git add src-tauri/src/space.rs src-tauri/src/lib.rs
git commit -m "feat: add free-space and cloud-sync-path preflight logic"
```

---

### Task 9: `adbsync --show-progress` output parser (Rust, TDD)

**Files:**
- Create: `src-tauri/src/sync/progress_parser.rs`
- Modify: `src-tauri/src/sync/mod.rs` (create if it doesn't exist yet — add `pub mod path_pair; pub mod progress_parser;`)

**Step 1: Write the failing tests, using canned output including a permission-denied line**

```rust
// src-tauri/src/sync/progress_parser.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_file_copied_line() {
        let events = parse_line("Copying DCIM/Camera/IMG_0001.jpg");
        assert_eq!(events, Some(ProgressEvent::Copying { path: "DCIM/Camera/IMG_0001.jpg".into() }));
    }

    #[test]
    fn parses_a_critical_error_line_as_a_fatal_event() {
        let events = parse_line("[CRITICAL] ADB line not captured");
        assert_eq!(events, Some(ProgressEvent::Fatal { message: "ADB line not captured".into() }));
    }

    #[test]
    fn surfaces_permission_denied_lines_instead_of_swallowing_them() {
        let events = parse_line("ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied");
        assert_eq!(
            events,
            Some(ProgressEvent::Error { message: "ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied".into() })
        );
    }

    #[test]
    fn ignores_blank_lines() {
        assert_eq!(parse_line(""), None);
    }
}
```

**Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test progress_parser::`
Expected: FAIL — types/functions not found.

**Step 3: Implement**

```rust
// src-tauri/src/sync/progress_parser.rs (above the #[cfg(test)] module)
#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum ProgressEvent {
    Copying { path: String },
    Fatal { message: String },
    Error { message: String },
}

pub fn parse_line(line: &str) -> Option<ProgressEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("[CRITICAL] ") {
        return Some(ProgressEvent::Fatal { message: rest.to_string() });
    }
    if line.starts_with("ls:") && line.ends_with("Permission denied") {
        return Some(ProgressEvent::Error { message: line.to_string() });
    }
    if let Some(rest) = line.strip_prefix("Copying ") {
        return Some(ProgressEvent::Copying { path: rest.to_string() });
    }
    None
}
```

Note for whoever implements this: the exact stdout line formats above
(`"Copying <path>"`, `"[CRITICAL] ..."`) should be double-checked against a
real `adbsync --show-progress` run and against
`third_party/better-adb-sync/src/BetterADBSync/SAOLogging.py` — this task
encodes the shapes described in `docs/research/adbsync-tooling.md` and the
session transcript, but hasn't been byte-verified against live stdout yet.
Add a step here to capture real output from a manual dry run and reconcile.

**Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test progress_parser::`
Expected: `4 passed`.

**Step 5: Commit**

```bash
git add src-tauri/src/sync/progress_parser.rs src-tauri/src/sync/mod.rs
git commit -m "feat: add adbsync output parser for progress/error events"
```

---

### Task 10: `sync::run_backup` / `sync::run_restore` orchestration + DB writes

**Files:**
- Create: `src-tauri/src/sync/mod.rs` (extend from Task 9)
- Modify: `src-tauri/src/lib.rs`

**Step 1: Write the integration-style test for the orchestration loop, using an injected fake runner**

```rust
// src-tauri/src/sync/mod.rs — add near the bottom
#[cfg(test)]
mod orchestration_tests {
    use super::*;

    struct FakeRunner {
        results: Vec<Result<(), String>>,
    }

    impl FolderSyncRunner for FakeRunner {
        fn run_one_folder(&mut self, _pair: &path_pair::PathPair) -> Result<(), String> {
            self.results.remove(0)
        }
    }

    #[test]
    fn stops_the_batch_on_first_failure_and_reports_which_folder() {
        let mut runner = FakeRunner { results: vec![Ok(()), Err("disk full".into()), Ok(())] };
        let folders = vec!["DCIM", "Pictures", "Movies"];
        let outcome = run_batch(&folders, &mut runner);

        assert_eq!(outcome.completed, vec!["DCIM"]);
        assert_eq!(outcome.failed_at, Some(("Pictures".to_string(), "disk full".to_string())));
        // Movies never attempted
    }

    #[test]
    fn all_succeed_marks_the_whole_batch_complete() {
        let mut runner = FakeRunner { results: vec![Ok(()), Ok(())] };
        let folders = vec!["DCIM", "Pictures"];
        let outcome = run_batch(&folders, &mut runner);

        assert_eq!(outcome.completed, vec!["DCIM", "Pictures"]);
        assert_eq!(outcome.failed_at, None);
    }
}
```

**Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test orchestration_tests`
Expected: FAIL — `FolderSyncRunner`/`run_batch`/`BatchOutcome` not found.

**Step 3: Implement the runner trait and batch loop**

```rust
// src-tauri/src/sync/mod.rs
pub mod path_pair;
pub mod progress_parser;

pub trait FolderSyncRunner {
    fn run_one_folder(&mut self, pair: &path_pair::PathPair) -> Result<(), String>;
}

pub struct BatchOutcome {
    pub completed: Vec<String>,
    pub failed_at: Option<(String, String)>, // (folder name, error message)
}

pub fn run_batch(folders: &[&str], runner: &mut impl FolderSyncRunner) -> BatchOutcome {
    let mut completed = Vec::new();
    for folder in folders {
        let pair = path_pair::PathPair { android: folder.to_string(), local: folder.to_string() };
        match runner.run_one_folder(&pair) {
            Ok(()) => completed.push(folder.to_string()),
            Err(message) => return BatchOutcome { completed, failed_at: Some((folder.to_string(), message)) },
        }
    }
    BatchOutcome { completed, failed_at: None }
}
```

**Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test orchestration_tests`
Expected: `2 passed`.

**Step 5: Wire the real subprocess-spawning `FolderSyncRunner` and the Tauri commands**

This needs the sidecar wiring from Task 3 and the path-pair builder from
Task 5. Implement a `RealFolderSyncRunner` that spawns the `adbsync` sidecar
per folder (mirroring the `list_devices` sidecar-spawn pattern from Task 6),
feeds stdout through `progress_parser::parse_line`, emits a Tauri event per
`ProgressEvent`, and returns `Err` on any `ProgressEvent::Fatal`/`Error`.
Then add `#[tauri::command] async fn run_backup(...)` /
`async fn run_restore(...)` that call `run_batch` with it, and write
`runs`/`run_items` rows via the Task 4 `db` client (from the frontend side,
since Drizzle runs in the JS layer — the Tauri command should emit a
"batch finished" event with the full `BatchOutcome`, and the frontend writes
the `runs`/`run_items` rows in response, rather than Rust talking to SQLite
directly). Confirm this split (Rust owns subprocess + events, TS owns DB
writes) still matches the design in
`docs/plans/2026-08-12-android-backup-restore-sync-design.md` §4 before
proceeding — flag to the user if it doesn't and a Rust-side SQLite client
(e.g. `rusqlite`) would be simpler than routing DB writes through the
frontend.

**Step 6: Commit**

```bash
git add src-tauri/src/sync/mod.rs src-tauri/src/lib.rs
git commit -m "feat: add sync batch orchestration and run/restore Tauri commands"
```

---

### Task 11: Device screen

**Files:**
- Create: `src/screens/DeviceScreen.tsx`
- Modify: `src/App.tsx` (replace the placeholder `EmptyState` scaffold with routing to this screen)

**Step 1: Find the closest astryx kit before writing any JSX**

Run: `bunx astryx build "list connected devices and select one"`
Read the returned kit (closest page/blocks/components) — do not freehand the
layout. If it returns a List/Table-based kit, prefer that over inventing a
custom layout, per `.claude/CLAUDE.md`'s "dense data = rows" rule.

**Step 2: Check exact props for whichever components the kit names**

Run: `bunx astryx component <Name>` for each component the kit uses (e.g.
`List`, `Item`, `Button`, `EmptyState` for the zero-devices case) before
writing JSX — do not guess props, this bit us once already this session
(the `EmptyState` description-string JSX-escaping issue).

**Step 3: Implement the screen**

Call `invoke("list_devices")` (Task 6) on mount, render the returned devices
using the kit's pattern, empty state if none connected, selecting a device
navigates to Task 12 (new device) or Task 13 (known device, decided by
whether `profile::load_or_create` — not yet built, stub with "always new"
until Task 4's schema is wired to a `profile` Tauri command in a follow-up
task).

**Step 4: Manual verification**

Run `bun run tauri dev` with a device connected (and disconnected), confirm
both states render. Add to manual QA checklist (Task 16).

**Step 5: Commit**

```bash
git add src/screens/DeviceScreen.tsx src/App.tsx
git commit -m "feat: add device selection screen"
```

---

### Task 12: Classification screen

**Files:**
- Create: `src/screens/ClassificationScreen.tsx`

**Step 1: Find the closest astryx kit**

Run: `bunx astryx build "review and edit a checklist of folders to include or skip"`

**Step 2: Verify component props**

Run: `bunx astryx component <Name>` for whatever the kit returns (likely
`List`/`Item` with a toggle, or `Table` with a checkbox column, plus `Badge`
or `StatusDot` for flagging the stale-duplicate case — verify `Badge`'s
guidance in `.claude/CLAUDE.md` against "counts and enumerated states, never
decoration" before using it here).

**Step 3: Implement**

Calls a `classify_suggest` Tauri command (wraps Task 7's `classify` module —
add this Tauri command as part of this task if not already exposed), renders
suggested Include/Skip toggles pre-checked per the heuristic, flags stale
duplicates distinctly (not just pre-checked Skip — the design requires this
be visible, not silent), saves the reviewed list via a `profile_save` Tauri
command that persists `folder_rules` rows through the Task 4 `db` client.

**Step 4: Manual verification against the real test-device table**

Cross-check the rendered suggestions against the worked example table in
`docs/research/android-storage-domain.md` for the actual test device, if
available. Add to manual QA checklist (Task 16).

**Step 5: Commit**

```bash
git add src/screens/ClassificationScreen.tsx
git commit -m "feat: add folder classification review screen"
```

---

### Task 13: Run screen (backup + restore)

**Files:**
- Create: `src/screens/RunScreen.tsx`

**Step 1: Find the closest astryx kit**

Run: `bunx astryx build "show preflight checks then live per-folder progress for a long-running operation"`

**Step 2: Verify component props**

Run: `bunx astryx component <Name>` for whatever's returned (likely
`Banner` for the cloud-sync warning per `.claude/CLAUDE.md`'s guidance that
EmptyState isn't for warnings needing action, plus a progress
component and `StatusDot`/`Token` for per-folder status).

**Step 3: Implement**

Runs preflight (`space_check` Tauri command wrapping Task 8's logic plus a
real `adb shell du` estimate and real free-disk-space lookup), gates the
start button on the result (free-space warning blocks, cloud-sync warning
doesn't), then invokes `run_backup`/`run_restore` (Task 10) and subscribes to
its progress events, updating per-folder status live. On the "batch
finished" event, writes the `runs`/`run_items` rows per Task 10 Step 5's
resolved DB-write approach.

**Step 4: Manual verification**

Full backup round trip against the real test device, and a restore round
trip. Add both to the manual QA checklist (Task 16).

**Step 5: Commit**

```bash
git add src/screens/RunScreen.tsx
git commit -m "feat: add backup/restore run screen with live progress"
```

---

### Task 14: History view

**Files:**
- Create: `src/screens/HistoryScreen.tsx`

**Step 1: Find the closest astryx kit**

Run: `bunx astryx build "list of past runs with status, expandable to per-item detail"`

**Step 2: Verify component props, then implement**

Query `runs`/`run_items` via the Task 4 `db` client directly from the
frontend (per design §4 — no Tauri command needed, Drizzle talks to SQLite
through the plugin), render per the returned kit. Compute "last synced" and
"not-synced" per the design's derivation rules (§4) as plain query logic, not
new Rust code.

**Step 3: Commit**

```bash
git add src/screens/HistoryScreen.tsx
git commit -m "feat: add run history view"
```

---

### Task 15: Profile settings screen

**Files:**
- Create: `src/screens/ProfileSettingsScreen.tsx`

**Step 1: Find the closest astryx kit**

Run: `bunx astryx build "list of saved profiles, edit one's settings"`

**Step 2: Verify component props, then implement**

List saved `devices` rows, editing a profile reopens the Task 12
Classification screen pre-populated with its current `folder_rules`, plus a
destination-path field.

**Step 3: Commit**

```bash
git add src/screens/ProfileSettingsScreen.tsx
git commit -m "feat: add profile settings screen"
```

---

### Task 16: Manual QA checklist (not automated)

**Files:**
- Create: `docs/plans/2026-08-12-android-backup-restore-sync-manual-qa.md`

Write up the checklist accumulated from the "Manual verification" steps
above, plus these from the design's Testing section:
- Real device backup, full round trip.
- Real device restore, full round trip.
- Disk-full mid-run behavior: does the app stop cleanly with a clear error,
  not the "silently re-copies everything forever" symptom from
  `docs/research/incident-disk-full-nested-duplicates.md`?
- Cloud-sync destination (OneDrive) warning actually triggers and is
  dismissible.
- Re-running a completed backup is fast/incremental (mtime diffing working
  as expected).
- Nested-duplicate-folder bug does NOT recur (spot check the destination
  tree structure after a run — no `DCIM\DCIM`-style nesting).

Run through this checklist manually once Tasks 1-15 are done; this is not
part of CI.

**Commit:**
```bash
git add docs/plans/2026-08-12-android-backup-restore-sync-manual-qa.md
git commit -m "docs: add manual QA checklist for backup/restore feature"
```
