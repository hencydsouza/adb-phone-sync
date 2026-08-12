//! Fills the gap left by Task 7: `classify` (in `classify.rs`) is a pure
//! heuristic over already-provided [`classify::FolderEntry`] data -- it does
//! no I/O. Nothing in Tasks 1-11 ever actually ran `adb shell ls` against a
//! real device and turned the output into `FolderEntry`s. This module is
//! that missing I/O layer: it runs `adb shell ls -la` (via the bundled `adb`
//! sidecar, mirroring `devices::list_devices`'s hardened subprocess pattern
//! and `sync::orchestration`'s per-device targeting), parses the output, and
//! exposes a `classify_suggest` Tauri command that ties the listing to
//! `classify::classify_with_mtime_hint`.
//!
//! ## Simplifications (read before extending)
//!
//! - **Sibling depth for the stale-duplicate check.** The design doc/
//!   research doc describe a full recursive walk of `Android/media/<pkg>/...`.
//!   This module instead does exactly *two* levels: list `Android/media`
//!   (one `ls` call) to get package folders, then list each package folder
//!   (one `ls` call per package) to get its immediate contents. That matches
//!   every real example in `docs/research/android-storage-domain.md` (e.g.
//!   `Android/media/com.whatsapp/WhatsApp`) and is exactly what
//!   [`classify::FolderEntry`]'s doc comment says `siblings` needs (full
//!   paths shaped like `Android/media/<pkg>/<name>`). It will NOT surface a
//!   shadow copy nested any deeper than that (e.g.
//!   `Android/media/<pkg>/<subdir>/<name>`) -- considered an acceptable gap
//!   for this task's scope, since going deeper multiplies the number of
//!   sequential `adb shell ls` round-trips with no observed real-world case
//!   requiring it.
//! - **`is_empty` is always `false` for real entries.** Getting a reliable
//!   emptiness signal would need one more `adb shell ls` call per top-level
//!   folder (to see if it has any contents), which is a lot of extra
//!   round-trips for a single boolean. Empirically, on the test device, a
//!   genuinely empty directory (e.g. `Alarms`) prints only a `total 0` line
//!   with no entries at all -- but relying on that reliably would still cost
//!   one extra `ls` per top-level folder, which this task skips. This is a
//!   known limitation: `classify()`'s empty-check is just one signal among
//!   several (personal-media / stock-noise / always-excluded / stale-shadow
//!   checks don't depend on it), so folders that are *actually* empty but
//!   don't match any other Skip signal will default to `Include` here and
//!   need a human to skip them in the review screen instead of being
//!   pre-classified automatically.
//! - **The top-level `Android` entry itself is dropped**, not passed through
//!   `classify()`. It isn't personal content, and unlike every other
//!   top-level folder it deliberately contains a permanently-excluded
//!   subtree (`Android/data`, `Android/obb` -- both root-restricted and
//!   already handled by `classify::ALWAYS_EXCLUDED_PREFIXES`) alongside a
//!   subtree we WANT to look inside without ever backing up wholesale
//!   (`Android/media`, walked above purely for sibling data). Passing bare
//!   `"Android"` through `classify()` would fall through every check to the
//!   default `Include`, which is wrong -- it isn't a syncable leaf the way
//!   `DCIM` or `WhatsApp` are.
//! - **Loose top-level files are dropped.** The classification screen is
//!   about folders (per the task title and `classify::FolderEntry`'s shape);
//!   real device roots also have loose files sitting directly in
//!   `/storage/emulated/0` (e.g. stray `.mp3`s). Those are filtered out
//!   (`is_dir` false) before classification -- out of scope here.
//! - **Per-package listing failures are non-fatal.** If listing a single
//!   `Android/media/<pkg>` folder fails (e.g. permission denied on some
//!   OEM/app package), that package is just skipped for sibling purposes
//!   rather than failing the whole command -- it's best-effort supplementary
//!   data for one signal, not required for the top-level listing to
//!   succeed.
//! - **`gather_media_siblings` fans out with a bounded concurrency cap**
//!   (see [`MEDIA_SIBLING_CONCURRENCY`]) instead of spawning every package's
//!   `adb shell ls` unbounded or running them fully sequentially. A
//!   well-populated phone can have 20-50+ packages under `Android/media`,
//!   and awaiting those one at a time would add real latency to
//!   `classify_suggest`. `tokio::task::JoinSet` (the `rt` feature is
//!   already pulled in transitively by `tauri`/`sqlx`, and is now declared
//!   directly in `Cargo.toml` too) lets a bounded batch of packages list
//!   concurrently while keeping the existing per-package
//!   best-effort/non-fatal error handling intact: each spawned task returns
//!   a `Result`, and a failure or panic for one package still only drops
//!   that package's sibling data, never the whole command.

use std::time::Duration;

use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use crate::classify;

/// Bound on how long we wait for a single `adb shell ls -la <path>` call to
/// finish. Mirrors `devices::LIST_DEVICES_TIMEOUT`'s reasoning: this is a
/// cheap, local (well, over-USB) operation, so a few seconds is generous --
/// it exists purely to guarantee we never hang forever on a stuck sidecar.
const LS_TIMEOUT: Duration = Duration::from_secs(10);

const STORAGE_ROOT: &str = "/storage/emulated/0";

/// Cap on how many `Android/media/<pkg>` listings [`gather_media_siblings`]
/// runs concurrently. Bounds subprocess fan-out (each one spawns a real
/// `adb` sidecar process) while still cutting wall-clock latency
/// dramatically versus a fully sequential walk on devices with many
/// packages. Not tuned against a real many-package device -- chosen as a
/// conservative middle ground between "no concurrency" and "unbounded
/// concurrency".
const MEDIA_SIBLING_CONCURRENCY: usize = 6;

/// One entry from a parsed `ls -la` listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsEntry {
    pub name: String,
    pub is_dir: bool,
    /// `"<date> <time>"` as reported by `ls -la` (e.g. `"2024-04-18 21:00"`).
    /// This format sorts correctly with a plain string comparison, which is
    /// all the stale-duplicate mtime check needs -- no date-parsing crate
    /// required.
    pub mtime: String,
}

/// Parse the stdout of `adb shell ls -la <dir>` (toybox `ls` on Android, as
/// verified against a real connected device) into a list of entries.
///
/// Known simplifications: this reconstructs each entry's name by joining
/// whitespace-separated tokens after the 7 fixed columns (permissions,
/// link-count, owner, group, size, date, time) with a single space. A name
/// with multiple *consecutive* internal spaces would be collapsed to one --
/// not observed in real device output and considered an acceptable
/// simplification. Symlink entries (`name -> target`) have the arrow and
/// target stripped, keeping just `name`.
pub fn parse_ls_la_output(raw: &str) -> Vec<LsEntry> {
    raw.lines().filter_map(parse_ls_la_line).collect()
}

fn parse_ls_la_line(line: &str) -> Option<LsEntry> {
    if line.starts_with("total ") {
        return None;
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    // perms, links, owner, group, size, date, time, then >=1 name token.
    if tokens.len() < 8 {
        return None;
    }
    let perms = tokens[0];
    // A real permission string is always 10 chars (e.g. "drwxrws---"). This
    // guards against accidentally treating a mixed-in stderr line (e.g.
    // "ls: /path: Permission denied") as a valid entry -- "ls:" is short
    // enough to never pass this check.
    if perms.len() != 10 {
        return None;
    }
    let is_dir = perms.starts_with('d');
    let date = tokens[5];
    let time = tokens[6];
    let mut name = tokens[7..].join(" ");
    if let Some(idx) = name.find(" -> ") {
        name.truncate(idx);
    }
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(LsEntry {
        name,
        is_dir,
        mtime: format!("{date} {time}"),
    })
}

/// Run `adb -s <serial> shell ls -la <path>` via the bundled `adb` sidecar
/// and return its raw stdout on success.
///
/// Mirrors `devices::list_devices`'s hardened subprocess pattern: full-drain
/// the event channel (never return early on `Terminated` -- see that
/// function's comment for why), bound the whole wait with a timeout, and
/// surface the real stderr/exit-code on failure instead of silently
/// returning nothing.
async fn run_adb_shell_ls(
    app: &tauri::AppHandle,
    serial: &str,
    path: &str,
) -> Result<String, String> {
    let (mut rx, child) = app
        .shell()
        .sidecar("adb")
        .map_err(|e| e.to_string())?
        .args(["-s", serial, "shell", "ls", "-la", path])
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    let receive_result = tokio::time::timeout(LS_TIMEOUT, async {
        let mut exit_code: Option<Option<i32>> = None;
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    stdout.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Stderr(bytes) => {
                    stderr.push_str(&String::from_utf8_lossy(&bytes));
                }
                CommandEvent::Error(err) => {
                    stderr.push_str(&err);
                }
                CommandEvent::Terminated(payload) => {
                    exit_code = Some(payload.code);
                }
                _ => {}
            }
        }
        exit_code
    })
    .await;

    match receive_result {
        Ok(Some(Some(0))) => Ok(stdout),
        Ok(Some(code)) => Err(format!(
            "adb shell ls -la {path} exited with code {code:?}: {}",
            stderr.trim()
        )),
        Ok(None) => Err(format!(
            "adb shell ls -la {path} process ended unexpectedly: {}",
            stderr.trim()
        )),
        Err(_) => {
            let _ = child.kill();
            Err(format!(
                "timed out waiting for `adb shell ls -la {path}` to respond"
            ))
        }
    }
}

/// A top-level folder plus its suggested classification, returned to the
/// frontend by [`classify_suggest`].
#[derive(Debug, serde::Serialize)]
pub struct SuggestedFolder {
    pub name: String,
    pub decision: classify::Decision,
}

/// Walk `Android/media` two levels deep (see module doc comment) and return
/// the resulting sibling `FolderEntry`s alongside their raw mtimes (needed
/// separately because `classify::FolderEntry` doesn't carry a timestamp).
/// Best-effort: a missing/unreadable `Android/media` or individual package
/// folder just yields less sibling data, not an error.
///
/// Package listings are fanned out in batches of up to
/// [`MEDIA_SIBLING_CONCURRENCY`] concurrent `adb shell ls` calls (via
/// `tokio::task::JoinSet`) rather than one at a time, to keep
/// `classify_suggest` responsive on devices with many packages under
/// `Android/media`. A failed or panicking task for one package is caught
/// and simply drops that package's sibling data -- it never fails the
/// batch or the overall command, preserving the same non-fatal semantics
/// the previous fully-sequential version had.
async fn gather_media_siblings(
    app: &tauri::AppHandle,
    serial: &str,
) -> (Vec<classify::FolderEntry>, Vec<(String, String)>) {
    let mut siblings = Vec::new();
    let mut sibling_mtimes = Vec::new();

    let media_path = format!("{STORAGE_ROOT}/Android/media");
    let Ok(media_raw) = run_adb_shell_ls(app, serial, &media_path).await else {
        return (siblings, sibling_mtimes);
    };

    let pkg_dirs: Vec<LsEntry> = parse_ls_la_output(&media_raw)
        .into_iter()
        .filter(|e| e.is_dir)
        .collect();

    for chunk in pkg_dirs.chunks(MEDIA_SIBLING_CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for pkg in chunk {
            let app = app.clone();
            let serial = serial.to_string();
            let pkg_path = format!("{media_path}/{}", pkg.name);
            let pkg_name = pkg.name.clone();
            set.spawn(async move {
                let result = run_adb_shell_ls(&app, &serial, &pkg_path).await;
                (pkg_name, result)
            });
        }

        while let Some(joined) = set.join_next().await {
            // `joined` is `Err` only if the spawned task panicked -- treat
            // that the same as a failed listing (best-effort, non-fatal).
            let Ok((pkg_name, pkg_result)) = joined else {
                continue;
            };
            let Ok(pkg_raw) = pkg_result else {
                continue;
            };
            for item in parse_ls_la_output(&pkg_raw) {
                let full_path = format!("Android/media/{pkg_name}/{}", item.name);
                sibling_mtimes.push((full_path.clone(), item.mtime));
                siblings.push(classify::FolderEntry {
                    name: full_path,
                    is_dir: item.is_dir,
                    is_empty: false,
                });
            }
        }
    }

    (siblings, sibling_mtimes)
}

/// List the profile root, apply `classify`'s heuristics (using a two-level
/// `Android/media` walk for the stale-duplicate signal), and return a
/// suggested Include/Skip/SkipStaleDuplicate decision per top-level folder.
/// See the module doc comment for every simplification made along the way.
#[tauri::command]
pub async fn classify_suggest(
    app: tauri::AppHandle,
    serial: String,
) -> Result<Vec<SuggestedFolder>, String> {
    let top_raw = run_adb_shell_ls(&app, &serial, STORAGE_ROOT).await?;
    let top_entries = parse_ls_la_output(&top_raw);

    let (siblings, sibling_mtimes) = gather_media_siblings(&app, &serial).await;

    let results = top_entries
        .into_iter()
        .filter(|entry| entry.is_dir && entry.name != "Android")
        .map(|entry| {
            let shadow_suffix = format!("/{}", entry.name);
            // Bare lexicographic string comparison, relying on `LsEntry::mtime`
            // being `"<date> <time>"` with zero-padded, fixed-width components
            // (as toybox `ls -la` prints them -- see that field's doc comment).
            // Holds for every real fixture captured so far; would need an
            // actual date-parsing comparison if a non-zero-padded `ls`
            // implementation were ever encountered.
            let is_newer_than_shadow = sibling_mtimes
                .iter()
                .find(|(path, _)| {
                    path.starts_with("Android/media/") && path.ends_with(&shadow_suffix)
                })
                .map(|(_, shadow_mtime)| entry.mtime.as_str() >= shadow_mtime.as_str())
                .unwrap_or(true);

            let folder_entry = classify::FolderEntry {
                name: entry.name.clone(),
                is_dir: entry.is_dir,
                is_empty: false,
            };
            let decision =
                classify::classify_with_mtime_hint(&folder_entry, &siblings, is_newer_than_shadow);
            SuggestedFolder {
                name: entry.name,
                decision,
            }
        })
        .collect();

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured verbatim from `adb -s 00070344C000047 shell ls -la
    /// /storage/emulated/0` against the real test device referenced in
    /// `docs/research/android-storage-domain.md` (same device serial), while
    /// building this task. Exercises: loose top-level files mixed with
    /// dirs, a directory name with an internal space ("Adobe Rush" -- not
    /// present at the top level but see the nested fixture below), varying
    /// link-counts/sizes, and the exact `WhatsApp` stale-duplicate scenario
    /// the research doc documents.
    const REAL_TOP_LEVEL: &str = "\
total 37789
drwxrws---  2 u0_a220  media_rw     3452 2024-11-24 22:34 Alarms
drwxrws--x  5 media_rw media_rw     3452 2024-11-24 22:34 Android
drwxrws---  2 u0_a220  media_rw     3452 2024-11-24 22:34 Audiobooks
-rw-rw----  1 u0_a220  media_rw  2581477 2019-04-11 06:23 Avicii - SOS (Fan Memories Video) ft. Aloe Blacc.mp3
drwxrws--- 15 u0_a220  media_rw     3452 2026-07-26 02:40 DCIM
drwxrws---  2 u0_a220  media_rw     3452 2025-10-13 20:41 Dartotsu
drwxrws---  8 u0_a220  media_rw     3452 2026-07-26 00:58 Documents
drwxrws--- 27 u0_a220  media_rw    57344 2026-08-08 09:43 Download
drwxrws---  4 u0_a220  media_rw     3452 2026-03-14 19:03 Gcam
-rw-rw----  1 u0_a220  media_rw 31144194 2019-09-22 01:44 HEVENS(Final).wav
-rw-rw----  1 u0_a220  media_rw  1678385 2019-08-03 05:30 KENZO.MP3
drwxrws---  4 u0_a220  media_rw     3452 2025-08-09 13:08 Mihon
drwxrws---  6 u0_a220  media_rw     3452 2026-05-23 01:35 Movies
drwxrws---  6 u0_a220  media_rw     3452 2026-01-29 00:07 Music
drwxrws---  2 u0_a220  media_rw     3452 2025-09-14 18:08 Notifications
drwxrws--- 46 u0_a220  media_rw    20480 2026-06-16 11:22 Pictures
drwxrws---  2 u0_a220  media_rw     3452 2024-11-24 22:34 Podcasts
drwxrws---  2 u0_a220  media_rw     3452 2024-11-24 22:34 Recordings
drwxrws---  3 u0_a220  media_rw     3452 2025-02-14 00:11 Ringtones
drwxrws---  3 u0_a220  media_rw     3452 2024-04-20 18:15 SGCAM
drwxrws---  2 u0_a220  media_rw     3452 2025-03-02 10:41 SUYU
drwxrws---  2 u0_a220  media_rw     3452 2025-03-26 09:39 Stuff
drwxrws---  5 u0_a220  media_rw     3452 2026-08-10 23:27 SwiftBackup
-rw-rw----  1 u0_a220  media_rw  3118188 2019-04-24 02:38 Tungevaag & Raaban - Million Lights (Official Lyric Video).mp3
drwxrws---  3 u0_a220  media_rw     3452 2024-04-18 21:00 WhatsApp
";

    /// Captured from `adb shell ls -la /storage/emulated/0/Android/media`.
    const REAL_MEDIA_LEVEL: &str = "\
total 18
drwxrws--- 3 u0_a220 media_rw 3452 2025-06-10 16:26 com.Slack
drwxrws--- 2 u0_a220 media_rw 3452 2026-08-08 18:23 com.instagram.android
drwxrws--- 2 u0_a220 media_rw 3452 2025-07-29 20:14 com.openai.chatgpt
drwxrws--- 3 u0_a220 media_rw 3452 2022-06-09 22:08 com.whatsapp
drwxrws--- 2 u0_a220 media_rw 3452 2026-08-10 23:27 org.swiftapps.swiftbackup
drwxrws--- 3 u0_a220 media_rw 3452 2025-10-29 00:17 org.telegram.messenger
";

    /// Captured from `adb shell ls -la
    /// /storage/emulated/0/Android/media/com.whatsapp` -- the newer shadow
    /// copy of root `/WhatsApp`.
    const REAL_WHATSAPP_MEDIA_LEVEL: &str = "\
total 3
drwxrws--- 9 u0_a220 media_rw 3452 2025-10-29 06:47 WhatsApp
";

    /// Captured from a genuinely empty directory
    /// (`/storage/emulated/0/Alarms`) -- toybox `ls -la` prints only the
    /// `total 0` header, no dot-entries, for an empty dir on this device.
    const REAL_EMPTY_DIR: &str = "total 0\n";

    /// Captured from `/storage/emulated/0/Ringtones/Compositions`, used to
    /// verify a filename with an internal space parses correctly.
    const REAL_NESTED_WITH_SPACE_IN_NAME: &str = "\
total 1840
-rwxrwx--- 1 u0_a220 media_rw 1645677 2025-09-25 22:23 Invisible .ogg
-rwxrwx--- 1 u0_a220 media_rw  127029 2025-02-14 00:11 SAMPHA.ogg
-rwxrwx--- 1 u0_a220 media_rw  103688 2025-02-14 00:11 Swedish House Mafia.ogg
";

    #[test]
    fn parses_real_top_level_listing() {
        let entries = parse_ls_la_output(REAL_TOP_LEVEL);
        // 25 dirs + files; the "total" line is skipped and there are no
        // dot-entries at the top level.
        assert_eq!(entries.len(), 25);

        let dcim = entries.iter().find(|e| e.name == "DCIM").unwrap();
        assert!(dcim.is_dir);
        assert_eq!(dcim.mtime, "2026-07-26 02:40");

        let mp3 = entries
            .iter()
            .find(|e| e.name == "Avicii - SOS (Fan Memories Video) ft. Aloe Blacc.mp3")
            .unwrap();
        assert!(!mp3.is_dir);

        let whatsapp = entries.iter().find(|e| e.name == "WhatsApp").unwrap();
        assert!(whatsapp.is_dir);
        assert_eq!(whatsapp.mtime, "2024-04-18 21:00");

        assert!(entries.iter().any(|e| e.name == "Android" && e.is_dir));
    }

    #[test]
    fn parses_empty_directory_as_zero_entries() {
        assert_eq!(parse_ls_la_output(REAL_EMPTY_DIR), Vec::new());
    }

    #[test]
    fn parses_filename_with_internal_space() {
        let entries = parse_ls_la_output(REAL_NESTED_WITH_SPACE_IN_NAME);
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.name == "Invisible .ogg"));
        assert!(entries.iter().any(|e| e.name == "Swedish House Mafia.ogg"));
    }

    #[test]
    fn parses_media_package_listing() {
        let entries = parse_ls_la_output(REAL_MEDIA_LEVEL);
        assert_eq!(entries.len(), 6);
        assert!(entries.iter().all(|e| e.is_dir));
        assert!(entries.iter().any(|e| e.name == "com.whatsapp"));
    }

    /// End-to-end (minus the actual subprocess spawn) reproduction of the
    /// exact worked example in `docs/research/android-storage-domain.md`:
    /// root `/WhatsApp` (older, 2024-04-18) is shadowed by a newer copy at
    /// `Android/media/com.whatsapp/WhatsApp` (2025-10-29), and should come
    /// out as `SkipStaleDuplicate`. This is the same logic
    /// `classify_suggest` runs, just assembled directly from parsed
    /// real-device fixtures instead of a live `adb` call.
    #[test]
    fn real_device_data_flags_root_whatsapp_as_stale_duplicate() {
        let top_entries = parse_ls_la_output(REAL_TOP_LEVEL);
        let media_pkgs = parse_ls_la_output(REAL_MEDIA_LEVEL);
        let whatsapp_pkg = media_pkgs.iter().find(|e| e.name == "com.whatsapp").unwrap();

        let mut siblings = Vec::new();
        let mut sibling_mtimes = Vec::new();
        for item in parse_ls_la_output(REAL_WHATSAPP_MEDIA_LEVEL) {
            let full_path = format!("Android/media/{}/{}", whatsapp_pkg.name, item.name);
            sibling_mtimes.push((full_path.clone(), item.mtime));
            siblings.push(classify::FolderEntry {
                name: full_path,
                is_dir: item.is_dir,
                is_empty: false,
            });
        }

        let root_whatsapp = top_entries.iter().find(|e| e.name == "WhatsApp").unwrap();
        let shadow_suffix = format!("/{}", root_whatsapp.name);
        let is_newer_than_shadow = sibling_mtimes
            .iter()
            .find(|(path, _)| path.starts_with("Android/media/") && path.ends_with(&shadow_suffix))
            .map(|(_, shadow_mtime)| root_whatsapp.mtime.as_str() >= shadow_mtime.as_str())
            .unwrap_or(true);
        assert!(
            !is_newer_than_shadow,
            "root WhatsApp (2024-04-18) should compare as older than the 2025-10-29 shadow copy"
        );

        let folder_entry = classify::FolderEntry {
            name: root_whatsapp.name.clone(),
            is_dir: root_whatsapp.is_dir,
            is_empty: false,
        };
        let decision =
            classify::classify_with_mtime_hint(&folder_entry, &siblings, is_newer_than_shadow);
        assert_eq!(decision, classify::Decision::SkipStaleDuplicate);
    }

    #[test]
    fn ignores_dot_entries_and_permission_denied_stderr_lines() {
        let raw = "total 615\n\
                    drwxrws--x 198 media_rw ext_data_rw 24576 2026-08-10 23:27 .\n\
                    drwxrws--x   5 media_rw media_rw     3452 2024-11-24 22:34 ..\n\
                    -rw-rw----   1 u0_a220  ext_data_rw     0 2024-11-24 22:34 .nomedia\n\
                    ls: /storage/emulated/0/Android/data/com.foo: Permission denied\n";
        let entries = parse_ls_la_output(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, ".nomedia");
    }
}
