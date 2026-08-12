//! Free-space and cloud-sync-path preflight checks for a backup/restore run.
//!
//! [`check_space`] and [`is_cloud_synced_path`] below are pure decision
//! logic (Task 8) -- zero I/O, so they can (and do) run under plain unit
//! tests with no device or filesystem involved.
//!
//! Everything below the `#[cfg(test)]` boundary of that original module is
//! the I/O layer this file gained later (Task 13): a real free-disk-space
//! lookup for the destination path (via the `fs4` crate --
//! `fs4::available_space`, wrapping `GetDiskFreeSpaceExW` on Windows) and a
//! real size estimate via `adb shell du -s <path>` per included folder,
//! mirroring `device_scan.rs`'s hardened subprocess pattern (full-drain, no
//! early return on `Terminated`, bounded timeout, real error surfacing).
//! [`space_check`] ties the two together into the Tauri command the Run
//! screen's preflight step calls.

use std::time::Duration;

use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

/// Result of comparing an estimated transfer size against the destination's
/// available free space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SpaceCheck {
    /// `true` when `free_bytes` is strictly greater than `estimated_bytes` —
    /// i.e. the transfer would leave at least some headroom on the
    /// destination volume. Exactly-equal counts as NOT enough space: a
    /// transfer that consumes every last free byte leaves zero margin for
    /// filesystem metadata overhead, other concurrent writes, or the
    /// estimate being slightly off, so it's treated as a preflight failure
    /// rather than a razor-thin pass.
    pub has_enough_space: bool,
    pub free_bytes: u64,
    pub estimated_bytes: u64,
}

/// Compare an estimated transfer size (`estimated_bytes`, e.g. from `adb
/// shell du` on the source) against the destination's available free space
/// (`free_bytes`, e.g. from a disk-space lookup on the local target). Pure
/// decision logic only — callers are responsible for obtaining both values.
pub fn check_space(estimated_bytes: u64, free_bytes: u64) -> SpaceCheck {
    SpaceCheck { has_enough_space: free_bytes > estimated_bytes, free_bytes, estimated_bytes }
}

/// Marker substrings that indicate a path lives inside a cloud-sync client's
/// managed folder. Writing large backup trees into one of these can trigger
/// slow/blocking hydration or sync churn (see
/// `docs/research/windows-gotchas.md`), so callers should warn the user
/// rather than silently proceeding.
const CLOUD_SYNC_MARKERS: &[&str] = &["OneDrive", "Dropbox", "Google Drive", "iCloudDrive"];

/// `true` if `path` appears to be inside a cloud-sync client's managed
/// folder, based on a substring match against [`CLOUD_SYNC_MARKERS`].
///
/// This is a raw, case-sensitive substring match with no path
/// canonicalization: a symlinked/junctioned cloud-sync folder under a
/// different name, or a differently-cased path, will silently produce a
/// false negative (no warning shown). Acceptable for a "warn, don't block"
/// preflight check, but callers should not assume this is a fully robust
/// detection.
pub fn is_cloud_synced_path(path: &str) -> bool {
    CLOUD_SYNC_MARKERS.iter().any(|marker| path.contains(marker))
}

/// Bound on how long we wait for a single `adb shell du -s <path>` call to
/// finish. Unlike `device_scan::LS_TIMEOUT`'s "cheap, single-directory
/// listing" reasoning, `du -s` recursively walks the whole folder tree
/// server-side, so this must tolerate large folders -- a real ~28GB DCIM
/// folder on the test device (00070344C000047) took ~9s. 3 minutes is a
/// generous but still-bounded ceiling for a folder far larger than that,
/// guaranteeing the command eventually resolves instead of hanging forever
/// on a stuck sidecar.
const DU_TIMEOUT: Duration = Duration::from_secs(3 * 60);

/// Cap on how many `adb shell du -s` calls [`estimate_size_bytes`] runs
/// concurrently across the included paths. Mirrors
/// `device_scan::MEDIA_SIBLING_CONCURRENCY`'s reasoning: bounds subprocess
/// fan-out (each spawns a real `adb` sidecar process) while cutting
/// wall-clock latency for profiles with several included top-level folders.
const DU_ESTIMATE_CONCURRENCY: usize = 4;

/// Parse the stdout of `adb shell du -s <path>` (toybox `du` on Android) into
/// a byte count.
///
/// Format verified against a real connected device
/// (`adb -s 00070344C000047 shell du -s /storage/emulated/0/Alarms` ->
/// `"4\t/storage/emulated/0/Alarms"`, i.e. `<size_in_KB>` then a tab then the
/// path). Only the first line/token is read -- `du -s` (summarize, don't
/// recurse into a per-subfolder breakdown) prints exactly one summary line
/// per invocation.
pub fn parse_du_output(raw: &str, path: &str) -> Result<u64, String> {
    let first_line = raw
        .lines()
        .next()
        .ok_or_else(|| format!("adb shell du -s {path} produced no output"))?;
    let kb_str = first_line.split_whitespace().next().ok_or_else(|| {
        format!("adb shell du -s {path} produced an unparseable line: {first_line:?}")
    })?;
    let kb: u64 = kb_str.parse().map_err(|_| {
        format!("adb shell du -s {path} produced a non-numeric size {kb_str:?} (full line: {first_line:?})")
    })?;
    Ok(kb.saturating_mul(1024))
}

/// Run `adb -s <serial> shell du -s <path>` via the bundled `adb` sidecar
/// and return the folder's size in bytes.
///
/// Mirrors `device_scan::run_adb_shell_ls`'s hardened subprocess pattern:
/// full-drain the event channel (never return early on a `Terminated`
/// event -- see that function's doc comment, and `sync::orchestration`'s,
/// for why: `Terminated` races the stdout/stderr pipe-reader threads over
/// the same channel), bound the whole wait with a timeout, and surface the
/// real stderr/exit-code on failure instead of silently returning nothing.
async fn run_adb_shell_du(
    app: &tauri::AppHandle,
    serial: &str,
    path: &str,
) -> Result<u64, String> {
    let (mut rx, child) = app
        .shell()
        .sidecar("adb")
        .map_err(|e| e.to_string())?
        .args(["-s", serial, "shell", "du", "-s", path])
        .spawn()
        .map_err(|e| e.to_string())?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    let receive_result = tokio::time::timeout(DU_TIMEOUT, async {
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
        Ok(exit_code) => interpret_du_exit(exit_code, &stdout, &stderr, path),
        Err(_) => {
            let _ = child.kill();
            Err(format!(
                "timed out waiting for `adb shell du -s {path}` to respond"
            ))
        }
    }
}

/// Interpret the drained `adb shell du -s <path>` event-channel outcome
/// (exit code, stdout, stderr) into a byte count or an error. Pulled out of
/// [`run_adb_shell_du`] as a pure function so this decision logic -- notably
/// the defensive stderr-even-on-exit-0 check below -- can be unit tested
/// without a real `AppHandle`/subprocess.
///
/// Defensive: don't trust exit code 0 alone. `du -s` walking into a
/// permission-restricted subtree (e.g. `Android/data`/`Android/obb` -- see
/// `docs/research/adbsync-tooling.md`'s Bug 2, and `progress_parser.rs`'s
/// handling of the same underlying `Permission denied` condition) may route
/// the error to stderr only, without necessarily causing a non-zero exit --
/// toybox `du`'s exit code is not guaranteed to reflect a partial walk.
/// Treating any captured stderr as a failure, even alongside a clean exit
/// code, guards against silently under-counting `estimated_bytes` with a
/// partial total instead of failing the whole estimate as documented on
/// [`estimate_size_bytes`].
fn interpret_du_exit(
    exit_code: Option<Option<i32>>,
    stdout: &str,
    stderr: &str,
    path: &str,
) -> Result<u64, String> {
    match exit_code {
        Some(Some(0)) if !stderr.trim().is_empty() => Err(format!(
            "adb shell du -s {path} exited 0 but reported errors on stderr (likely a partial/inaccurate size due to a permission-restricted subfolder): {}",
            stderr.trim()
        )),
        Some(Some(0)) => parse_du_output(stdout, path),
        Some(code) => Err(format!(
            "adb shell du -s {path} exited with code {code:?}: {}",
            stderr.trim()
        )),
        None => Err(format!(
            "adb shell du -s {path} process ended unexpectedly: {}",
            stderr.trim()
        )),
    }
}

/// Sum `adb shell du -s` estimates across every included path, fanning out
/// up to [`DU_ESTIMATE_CONCURRENCY`] concurrent calls at a time (per-chunk;
/// each chunk is fully awaited before the next one starts).
///
/// Unlike `device_scan::gather_media_siblings`'s best-effort sibling data
/// (where a failed listing just means less supplementary sibling data for
/// one classification signal), a failure here fails the *whole* estimate:
/// `estimated_bytes` is what gates the free-space preflight check, so
/// silently under-counting a folder that failed to report its size could
/// let a transfer proceed straight into the disk-full scenario the
/// preflight check exists to catch (design doc §5). Real error surfacing,
/// not best-effort, is the correct behavior here.
pub async fn estimate_size_bytes(
    app: &tauri::AppHandle,
    serial: &str,
    included_paths: &[String],
) -> Result<u64, String> {
    let mut total: u64 = 0;
    for chunk in included_paths.chunks(DU_ESTIMATE_CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for path in chunk {
            let app = app.clone();
            let serial = serial.to_string();
            let path = path.clone();
            set.spawn(async move { run_adb_shell_du(&app, &serial, &path).await });
        }
        while let Some(joined) = set.join_next().await {
            let result = joined.map_err(|e| e.to_string())?;
            total = total.saturating_add(result?);
        }
    }
    Ok(total)
}

/// Real free-disk-space lookup for the destination folder, via the `fs4`
/// crate's `available_space` (`GetDiskFreeSpaceExW` under the hood on
/// Windows -- caller-scoped free space, which honours per-user quotas; see
/// that crate's `windows.rs::statvfs`).
///
/// `GetDiskFreeSpaceExW` is resolved against the destination's containing
/// *volume* (via `GetVolumePathNameW`), which works even for a path that
/// doesn't exist on disk yet -- e.g. a destination folder the user has
/// picked but this app hasn't created yet -- as long as the drive/volume
/// itself exists. Confirmed by reading the `fs4` v1.1.0 source directly
/// (`windows.rs`), not assumed.
///
/// This is a fast, local, synchronous syscall, but it's still routed
/// through `spawn_blocking` since it's called from an async Tauri command
/// and no blocking syscall is guaranteed instant (e.g. a network drive
/// destination).
pub async fn free_space_bytes(dest: &str) -> Result<u64, String> {
    let dest = dest.to_string();
    tokio::task::spawn_blocking(move || {
        fs4::available_space(&dest)
            .map_err(|e| format!("failed to read free disk space for \"{dest}\": {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Result of [`space_check`], returned to the frontend as one JSON object:
/// the pure [`SpaceCheck`] decision (free vs. estimated bytes) plus the
/// independent cloud-sync-path warning signal. Kept as two separate
/// booleans/fields rather than folded into one "ok/not ok" flag because the
/// Run screen's preflight gating treats them differently (design doc §5):
/// `has_enough_space == false` blocks the start button, `is_cloud_synced ==
/// true` only shows a dismissible warning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpaceCheckResult {
    pub has_enough_space: bool,
    pub free_bytes: u64,
    pub estimated_bytes: u64,
    pub is_cloud_synced: bool,
}

/// Tauri command backing the Run screen's preflight check: gathers the real
/// numbers (`adb shell du -s` per included path, summed; real free-space
/// lookup on `dest`) and runs them through the pure [`check_space`] /
/// [`is_cloud_synced_path`] decision logic from the top of this file.
#[tauri::command]
pub async fn space_check(
    app: tauri::AppHandle,
    serial: String,
    dest: String,
    included_paths: Vec<String>,
) -> Result<SpaceCheckResult, String> {
    let estimated_bytes = estimate_size_bytes(&app, &serial, &included_paths).await?;
    let free_bytes = free_space_bytes(&dest).await?;
    let check = check_space(estimated_bytes, free_bytes);
    let is_cloud_synced = is_cloud_synced_path(&dest);
    Ok(SpaceCheckResult {
        has_enough_space: check.has_enough_space,
        free_bytes: check.free_bytes,
        estimated_bytes: check.estimated_bytes,
        is_cloud_synced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_when_free_space_is_below_estimate() {
        let result = check_space(10_000, 5_000);
        assert!(!result.has_enough_space);
    }

    #[test]
    fn passes_when_free_space_exceeds_estimate() {
        let result = check_space(5_000, 10_000);
        assert!(result.has_enough_space);
    }

    #[test]
    fn fails_when_free_space_exactly_equals_estimate() {
        // Exactly-equal is an intentional design decision (strict `>`, not
        // `>=`): consuming every last free byte leaves zero margin for
        // filesystem metadata overhead or estimate drift, so it must be
        // treated as NOT enough space.
        let result = check_space(10_000, 10_000);
        assert!(!result.has_enough_space);
    }

    /// Genuine end-to-end coverage of `free_space_bytes` (real `fs4`/Windows
    /// syscall, no mocking) -- unlike `run_adb_shell_du`/`space_check`,
    /// which need a real `tauri::AppHandle` to resolve the bundled `adb`
    /// sidecar (not practical to construct in a unit test), this function
    /// takes no `AppHandle` at all, so it can be exercised directly against
    /// this repo's own directory on whatever machine runs the test suite.
    #[tokio::test]
    async fn free_space_bytes_reports_a_plausible_value_for_a_real_path() {
        let dest = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let bytes = free_space_bytes(&dest).await.unwrap();
        // Not asserting an exact number (it'll drift as the real disk fills
        // up/frees up between test runs) -- just that a real, sane value
        // came back rather than 0 or an error.
        assert!(bytes > 0);
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

    #[test]
    fn detects_google_drive_paths() {
        assert!(is_cloud_synced_path(r"C:\Users\hency\Google Drive\phone"));
    }

    #[test]
    fn detects_icloud_drive_paths() {
        assert!(is_cloud_synced_path(r"C:\Users\hency\iCloudDrive\phone"));
    }

    #[test]
    fn parses_real_du_output_into_bytes() {
        // Captured verbatim from
        // `adb -s 00070344C000047 shell du -s /storage/emulated/0/Alarms`
        // against the real test device: "4\t/storage/emulated/0/Alarms".
        let bytes =
            parse_du_output("4\t/storage/emulated/0/Alarms\n", "/storage/emulated/0/Alarms")
                .unwrap();
        assert_eq!(bytes, 4 * 1024);
    }

    #[test]
    fn parses_a_larger_real_du_output() {
        // Captured verbatim from
        // `adb -s 00070344C000047 shell du -s /storage/emulated/0/DCIM`:
        // "28946919\t/storage/emulated/0/DCIM".
        let bytes =
            parse_du_output("28946919\t/storage/emulated/0/DCIM\n", "/storage/emulated/0/DCIM")
                .unwrap();
        assert_eq!(bytes, 28_946_919 * 1024);
    }

    #[test]
    fn du_output_with_no_lines_is_an_error() {
        assert!(parse_du_output("", "/storage/emulated/0/DCIM").is_err());
    }

    #[test]
    fn du_output_with_a_non_numeric_size_is_an_error() {
        assert!(parse_du_output(
            "du: /storage/emulated/0/NoSuchFolder: No such file or directory",
            "/storage/emulated/0/NoSuchFolder"
        )
        .is_err());
    }

    #[test]
    fn exit_zero_with_clean_stdout_and_no_stderr_succeeds() {
        // The ordinary, fully-accessible-folder case: exit 0, no stderr.
        let result = interpret_du_exit(
            Some(Some(0)),
            "28946919\t/storage/emulated/0/DCIM\n",
            "",
            "/storage/emulated/0/DCIM",
        );
        assert_eq!(result, Ok(28_946_919 * 1024));
    }

    /// Locks in the defensive fix for the real risk this whole module's
    /// "fail-whole-estimate" guarantee exists to guard against (see
    /// `docs/research/adbsync-tooling.md`'s Bug 2): `du -s` walking into a
    /// permission-restricted subtree like `Android/data`/`Android/obb` can
    /// exit 0 with a PARTIAL total on stdout while routing the permission
    /// error to stderr only -- many `du` implementations (including
    /// toybox's on Android) don't reliably turn that into a non-zero exit
    /// code. Without this check, `estimate_size_bytes` would silently
    /// UNDER-COUNT instead of failing, which is the opposite of what
    /// `estimate_size_bytes`'s doc comment claims to guarantee. Can't be
    /// live-tested without a real permission-restricted device path (not
    /// available in this sandboxed environment), so this simulates the
    /// scenario directly: zero exit code + non-empty stderr must still be
    /// treated as a failure, even though `stdout` alone would parse fine.
    #[test]
    fn exit_zero_with_stderr_output_is_treated_as_a_failure() {
        let result = interpret_du_exit(
            Some(Some(0)),
            "12345\t/storage/emulated/0\n",
            "du: /storage/emulated/0/Android/data/org.videolan.vlc/files: Permission denied\n",
            "/storage/emulated/0",
        );
        assert!(result.is_err());
        let message = result.unwrap_err();
        assert!(message.contains("exited 0 but reported errors on stderr"));
        assert!(message.contains("Permission denied"));
    }

    #[test]
    fn nonzero_exit_is_an_error_regardless_of_stderr_content() {
        let result = interpret_du_exit(
            Some(Some(1)),
            "",
            "du: /storage/emulated/0/NoSuchFolder: No such file or directory",
            "/storage/emulated/0/NoSuchFolder",
        );
        assert!(result.is_err());
    }

    #[test]
    fn process_ending_without_a_terminated_event_is_an_error() {
        let result = interpret_du_exit(None, "", "", "/storage/emulated/0/DCIM");
        assert!(result.is_err());
    }
}
