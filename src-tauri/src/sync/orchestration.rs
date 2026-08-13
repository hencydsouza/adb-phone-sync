//! Batch orchestration for a backup/restore run: iterate the included
//! folders one at a time (never a whole-tree call, per
//! `docs/research/adbsync-tooling.md` Bug 2), stop on the first failure, and
//! report which folder failed and why.
//!
//! Rust's job stops at "spawn the subprocess, parse its output, emit Tauri
//! events." It deliberately does NOT touch SQLite / write `runs`/`run_items`
//! rows -- that's a later task's job (the Run screen owns the frontend
//! Drizzle client). See Task 10 in
//! `docs/plans/2026-08-12-android-backup-restore-sync-implementation.md`.

use std::path::PathBuf;
use std::time::Duration;

use tauri::Emitter;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use super::path_pair;
use super::progress_parser::{self, ProgressEvent};

/// How long we'll wait for *any* new output from the `adbsync` subprocess
/// before treating it as hung. This is NOT a bound on total transfer time --
/// a legitimate backup can run for a long time copying gigabytes of photos,
/// so a flat overall timeout (like the one `devices::list_devices` uses for
/// its much smaller, bounded `adb devices -l` call) would be wrong here. It
/// resets on every event received from the subprocess (stdout/stderr/error),
/// so it only fires if the process goes completely silent for this long.
const SYNC_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Injected per-folder sync operation. The real implementation
/// ([`RealFolderSyncRunner`]) spawns the `adbsync` sidecar; tests inject a
/// fake to exercise [`run_batch`]'s sequencing/stop-on-failure behavior in
/// isolation.
///
/// This intentionally takes a plain folder identifier rather than a
/// [`path_pair::PathPair`]: `PathPair`'s fields are private by design (see
/// `path_pair.rs`'s doc comment -- it exists specifically to stop callers
/// from hand-constructing an ANDROID/LOCAL pair and reintroducing the
/// trailing-slash nesting bug), and building a real pair requires knowing
/// the sync *direction* (pull for backup, push for restore) plus the
/// profile's local destination root, none of which `run_batch` itself needs
/// or should know about. `run_batch`'s only job is sequencing and
/// stop-on-first-failure; direction-aware `PathPair` construction happens
/// inside `RealFolderSyncRunner`, via `path_pair::build_pull_pair`/
/// `build_push_pair`, right next to where the resulting paths are actually
/// used.
pub trait FolderSyncRunner {
    fn run_one_folder(&mut self, folder: &str) -> Result<(), String>;
}

/// The result of running a batch of folders: which ones finished, and, if
/// the batch stopped early, which folder failed and why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BatchOutcome {
    pub completed: Vec<String>,
    pub failed_at: Option<(String, String)>,
}

/// Run each folder in order, stopping at the first failure. Mirrors the
/// design doc's error-handling rule (§5): "the batch stops there... and
/// leaves already-completed folders synced" -- no blind retry, no
/// silent-continue past a failure.
pub fn run_batch(folders: &[&str], runner: &mut impl FolderSyncRunner) -> BatchOutcome {
    let mut completed = Vec::new();
    for folder in folders {
        match runner.run_one_folder(folder) {
            Ok(()) => completed.push(folder.to_string()),
            Err(message) => {
                return BatchOutcome {
                    completed,
                    failed_at: Some((folder.to_string(), message)),
                }
            }
        }
    }
    BatchOutcome {
        completed,
        failed_at: None,
    }
}

/// Which way data is moving for this batch. Backup pulls ANDROID -> LOCAL;
/// restore pushes LOCAL -> ANDROID. Per the design doc §3, these share the
/// same "one call per included leaf path" primitive with source/dest
/// swapped, not a separate implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    Backup,
    Restore,
}

/// Incrementally splits raw subprocess stdout/stderr byte chunks into
/// discrete lines on either `\n` or `\r`.
///
/// `adb`'s own progress display redraws in place using a bare `\r` (not
/// `\n`) between percentage updates for the same file; ordinary log lines
/// end in `\n`. The shell plugin hands us raw byte chunks as they arrive
/// from the OS pipe -- chunk boundaries line up with neither separator, so
/// a single chunk can contain zero, one, or many complete lines, and a line
/// can be split across two chunks.
///
/// Splitting on BOTH `\n` and `\r` here (rather than `\n` only) gives one
/// `parse_line` call per redraw update instead of letting several redraws
/// concatenate into a single string with embedded `\r`s -- finer-grained
/// live progress for the frontend. `progress_parser::parse_line`'s own
/// `rsplit('\r')` handling then becomes a defensive no-op for whatever we
/// hand it here rather than the primary defense; if a chunk boundary ever
/// does land mid-redraw such that a `\r` slips through anyway, that
/// existing handling still protects `parse_line`'s output.
#[derive(Default)]
struct LineSplitter {
    buffer: String,
}

impl LineSplitter {
    /// Feed a new chunk in; returns every complete line it produced
    /// (across this call and any buffered from previous calls).
    fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut lines = Vec::new();
        while let Some(idx) = self.buffer.find(['\n', '\r']) {
            // `idx` points at an ASCII separator, so idx + 1 is always a
            // valid char boundary to split on.
            let rest = self.buffer.split_off(idx + 1);
            let mut line = std::mem::replace(&mut self.buffer, rest);
            line.pop(); // drop the trailing separator itself
            lines.push(line);
        }
        lines
    }

    /// Call once the stream has closed, to flush a final line that never
    /// got a trailing separator (e.g. the process exited mid-line).
    fn take_remainder(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.buffer))
        }
    }
}

/// Payload for the `sync-folder-start` event.
#[derive(Clone, serde::Serialize)]
struct FolderStartPayload<'a> {
    folder: &'a str,
}

/// Payload for the `sync-progress` event.
#[derive(Clone, serde::Serialize)]
struct SyncProgressPayload<'a> {
    folder: &'a str,
    event: ProgressEvent,
}

/// Payload for the `sync-folder-success` event.
#[derive(Clone, serde::Serialize)]
struct FolderSuccessPayload<'a> {
    folder: &'a str,
}

/// Payload for the `sync-folder-failure` event.
#[derive(Clone, serde::Serialize)]
struct FolderFailurePayload<'a> {
    folder: &'a str,
    error: &'a str,
}

/// Payload for the final `sync-batch-complete` event.
#[derive(Clone, serde::Serialize)]
struct BatchCompletePayload {
    outcome: BatchOutcome,
}

/// Resolves the on-disk path to a bundled sidecar binary, mirroring the
/// (private) path-resolution logic `tauri_plugin_shell::process::Command`
/// uses internally for `Shell::sidecar()`. We need the literal path here
/// (not just the ability to spawn it) because `adbsync` takes
/// `--adb-bin <path>` and shells out to `adb` itself as a *separate*
/// subprocess -- passing bare `"adb"` would depend on the ambient PATH,
/// which `docs/research/adbsync-tooling.md` found unreliable for a packaged
/// app ("A packaged app should always pass `--adb-bin` explicitly rather
/// than relying on ambient PATH").
fn resolve_sidecar_path(name: &str) -> Result<PathBuf, String> {
    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe_path
        .parent()
        .ok_or_else(|| "current executable has no parent directory".to_string())?;
    // `cargo test`/`cargo run` put the test/dev binary in
    // `.../target/debug/deps` or `.../target/debug`; the bundled sidecar
    // sits next to the *app* binary, one level up from `deps`. Same
    // adjustment tauri-plugin-shell's own sidecar resolution makes.
    let base_dir = if exe_dir.ends_with("deps") {
        exe_dir.parent().unwrap_or(exe_dir)
    } else {
        exe_dir
    };
    let mut path = base_dir.join(name);
    #[cfg(windows)]
    {
        let already_exe = path.extension().is_some_and(|ext| ext == "exe");
        if !already_exe {
            path.as_mut_os_string().push(".exe");
        }
    }
    if !path.exists() {
        return Err(format!(
            "{name} sidecar not found at {} -- expected it bundled next to the app binary",
            path.display()
        ));
    }
    Ok(path)
}

/// The leaf (last path segment) of an ANDROID-side path, used to name the
/// per-folder local destination subfolder, e.g.
/// `/storage/emulated/0/DCIM` -> `DCIM`.
fn leaf_name(android_path: &str) -> &str {
    let trimmed = android_path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Decide whether a single folder's `adbsync` invocation counts as a
/// success or failure, given how the drain loop ended.
///
/// `exit_code` is the (already timeout-checked) result of draining the
/// subprocess: `Some(Some(code))` if `adbsync` reported an exit code,
/// `Some(None)` if it was terminated without one, `None` if the event
/// channel closed without ever seeing a `Terminated` event.
///
/// Deliberately NOT a pure function of the exit code alone: a zero exit
/// code is not on its own sufficient to call the folder a success, for two
/// separate reasons layered on top of each other:
///
/// 1. `progress_parser::parse_line`'s `Error` variant exists precisely for
///    lines (e.g. a bare, unwrapped `ls: ...: Permission denied`) that are
///    non-fatal to the *process* but still must be surfaced to the user
///    rather than silently dropped -- see that module's doc comment. If
///    `adbsync` exits 0 but we still observed one of those (or a `Fatal`)
///    during the drain, this reports the folder as failed using that
///    message.
/// 2. Even when nothing was recognized by `parse_line` as a notable
///    message, `stderr_tail` on an exit-0 process is still scanned for a
///    small, specific denylist of known-fatal substrings (see
///    [`KNOWN_FATAL_STDERR_SUBSTRINGS`]) -- not "any non-empty stderr", and
///    not an allowlist of "known-benign" patterns either. Both of those
///    were tried and both false-positived on real, successful runs during
///    manual QA (see git history on this function): first an allowlist of
///    `adbsync`'s own `[INFO]`/`[DEBUG]` log-level prefixes (missed `adb`'s
///    *own* native "N file(s) pulled" progress-summary lines, which go
///    straight to stderr with no `[LEVEL]` prefix at all, since
///    `adbsync` runs `adb push`/`pull` with inherited, uncaptured stdio in
///    `--show-progress` mode -- see `docs/research/adbsync-tooling.md`).
///    Trying to enumerate every benign stderr shape a third-party tool we
///    don't control might print is whack-a-mole. Denylisting the specific,
///    documented real failure text instead
///    (`docs/research/incident-disk-full-nested-duplicates.md`:
///    `` `adb: error: cannot write '...': Input/output error` ``;
///    `docs/research/windows-gotchas.md`: `` `No space left on device` ``)
///    is far more conservative: it only fails a folder that would
///    otherwise report `sync-folder-success` when its stderr contains one
///    of a few known, specific disk-full/IO-failure phrases, and otherwise
///    trusts the exit code -- exactly the signal `adb pull`/`push` itself
///    is expected to set correctly for its own file-transfer outcome.
///
/// Together these guarantee a partially-broken-but-zero-exit run never gets
/// reported as `sync-folder-success` (design doc §5: never silently report
/// success when something went wrong), without misreporting a clean run as
/// failed just because of stderr text this app doesn't recognize.
fn determine_folder_outcome(
    exit_code: Option<Option<i32>>,
    last_notable_message: Option<String>,
    stderr_tail: &str,
    subcommand: &str,
) -> Result<(), String> {
    match exit_code {
        Some(Some(0)) => match last_notable_message {
            Some(message) => Err(message),
            None if stderr_contains_known_fatal_text(stderr_tail) => Err(format!(
                "adbsync {subcommand} exited 0 but reported errors on stderr (likely a \
                 partial/failed transfer): {}",
                stderr_tail.trim()
            )),
            None => Ok(()),
        },
        Some(code) => Err(last_notable_message.unwrap_or_else(|| {
            format!(
                "adbsync {subcommand} exited with code {code:?}: {}",
                stderr_tail.trim()
            )
        })),
        None => Err(last_notable_message.unwrap_or_else(|| {
            format!(
                "adbsync {subcommand} process ended unexpectedly: {}",
                stderr_tail.trim()
            )
        })),
    }
}

/// Specific, documented real failure text (see `determine_folder_outcome`'s
/// doc comment) that a disk-full/IO failure can print to stderr while the
/// process still exits 0. Deliberately narrow and case-insensitive-matched
/// against the whole (lowercased) `stderr_tail`, not a per-line prefix
/// check -- unlike an allowlist of "known-benign" patterns, a short
/// denylist of "known-bad" phrases can't be defeated by some other tool
/// (`adb`, `adbsync`, a future toybox update) printing a new benign line
/// shape we haven't seen yet.
const KNOWN_FATAL_STDERR_SUBSTRINGS: &[&str] =
    &["no space left", "input/output error", "cannot write"];

fn stderr_contains_known_fatal_text(stderr_tail: &str) -> bool {
    let lower = stderr_tail.to_lowercase();
    KNOWN_FATAL_STDERR_SUBSTRINGS
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Real, subprocess-spawning [`FolderSyncRunner`]. Spawns the `adbsync`
/// sidecar once per folder, parses its stdout/stderr through
/// [`progress_parser::parse_line`], and emits Tauri events as it goes.
pub struct RealFolderSyncRunner {
    app: tauri::AppHandle,
    serial: String,
    dest_root: PathBuf,
    direction: SyncDirection,
    adb_bin_path: PathBuf,
}

impl RealFolderSyncRunner {
    pub fn new(
        app: tauri::AppHandle,
        serial: String,
        dest_root: String,
        direction: SyncDirection,
    ) -> Result<Self, String> {
        let adb_bin_path = resolve_sidecar_path("adb")?;
        Ok(Self {
            app,
            serial,
            dest_root: PathBuf::from(dest_root),
            direction,
            adb_bin_path,
        })
    }

    async fn run_one_folder_async(&mut self, folder: &str) -> Result<(), String> {
        let leaf = leaf_name(folder);
        let local_path = self.dest_root.join(leaf);
        let local_str = local_path.to_string_lossy().into_owned();

        // Build the correctly trailing-slash-terminated pair for this
        // direction, then pull the two strings `adbsync` actually wants as
        // positional args back out of it. This is the one place direction
        // meets `PathPair` construction -- see the trait doc comment above.
        let (subcommand, source_arg, dest_arg) = match self.direction {
            SyncDirection::Backup => {
                let pair = path_pair::build_pull_pair(folder, &local_str);
                ("pull", pair.android().to_string(), pair.local().to_string())
            }
            SyncDirection::Restore => {
                let pair = path_pair::build_push_pair(&local_str, folder);
                ("push", pair.local().to_string(), pair.android().to_string())
            }
        };

        let adb_bin_str = self.adb_bin_path.to_string_lossy().into_owned();

        let _ = self
            .app
            .emit("sync-folder-start", FolderStartPayload { folder });

        // NOTE: the `shell:allow-execute` scope entry in
        // capabilities/default.json only gates shell invocations initiated
        // from frontend JS through the shell plugin's JS API -- it does not
        // gate this Rust-side `Shell::sidecar()` call. See the identical
        // note (with more detail on why the `adb` entry uses unrestricted
        // `"args": true`) on `devices::list_devices`.
        let spawn_result = self
            .app
            .shell()
            .sidecar("adbsync")
            .map_err(|e| e.to_string())
            .and_then(|cmd| {
                cmd.args([
                    "--adb-bin",
                    &adb_bin_str,
                    "--adb-option",
                    "s",
                    &self.serial,
                    "--show-progress",
                    subcommand,
                    &source_arg,
                    &dest_arg,
                ])
                .spawn()
                .map_err(|e| e.to_string())
            });

        let (mut rx, child) = match spawn_result {
            Ok(v) => v,
            Err(message) => {
                let _ = self.app.emit(
                    "sync-folder-failure",
                    FolderFailurePayload {
                        folder,
                        error: &message,
                    },
                );
                return Err(message);
            }
        };

        let mut stdout_splitter = LineSplitter::default();
        let mut stderr_splitter = LineSplitter::default();
        let mut stderr_tail = String::new();
        let mut last_notable_message: Option<String> = None;
        let mut exit_code: Option<Option<i32>> = None;

        // Full-drain loop, mirroring `devices::list_devices`: `Terminated`
        // races the stdout/stderr pipe-reader threads over the same
        // channel, so we only *record* the exit code when we see it and
        // keep draining -- the loop only ends once `rx.recv()` returns
        // `None`, i.e. once every sender (pipe readers included) has been
        // dropped and all output has actually been received. We do NOT
        // return early on `Terminated`; Task 6's review found and fixed a
        // real race from doing exactly that.
        //
        // Unlike `list_devices`'s single flat timeout (fine for a small,
        // bounded `adb devices -l` call), each individual `recv()` here is
        // bounded by `SYNC_INACTIVITY_TIMEOUT` instead -- a multi-gigabyte
        // folder sync can legitimately run far longer than any fixed
        // overall timeout, but should still be producing output
        // periodically. Silence for the full inactivity window is treated
        // as a hang.
        let drain_result: Result<Option<Option<i32>>, ()> = loop {
            match tokio::time::timeout(SYNC_INACTIVITY_TIMEOUT, rx.recv()).await {
                Ok(Some(event)) => match event {
                    CommandEvent::Stdout(bytes) => {
                        let chunk = String::from_utf8_lossy(&bytes);
                        for line in stdout_splitter.push(&chunk) {
                            Self::handle_line(&self.app, folder, &line, &mut last_notable_message);
                        }
                    }
                    CommandEvent::Stderr(bytes) => {
                        let chunk = String::from_utf8_lossy(&bytes);
                        // `chunk` arrives pre-split on its line terminator
                        // (tauri-plugin-shell's pipe reader delivers one
                        // event per `\n`/`\r`-terminated line, terminator
                        // stripped) -- push a `\n` back in so `stderr_tail`
                        // stays newline-delimited for
                        // `stderr_contains_known_fatal_text`'s check,
                        // instead of concatenating every line into one
                        // run-on blob with no separator.
                        stderr_tail.push_str(&chunk);
                        stderr_tail.push('\n');
                        for line in stderr_splitter.push(&chunk) {
                            Self::handle_line(&self.app, folder, &line, &mut last_notable_message);
                        }
                    }
                    CommandEvent::Error(err) => {
                        stderr_tail.push_str(&err);
                        stderr_tail.push('\n');
                    }
                    CommandEvent::Terminated(payload) => {
                        exit_code = Some(payload.code);
                    }
                    _ => {}
                },
                Ok(None) => break Ok(exit_code),
                Err(_) => break Err(()),
            }
        };

        // Flush any trailing partial line left in either splitter once the
        // stream has actually closed (only meaningful on the non-timeout
        // path, but harmless either way).
        if let Some(line) = stdout_splitter.take_remainder() {
            Self::handle_line(&self.app, folder, &line, &mut last_notable_message);
        }
        if let Some(line) = stderr_splitter.take_remainder() {
            Self::handle_line(&self.app, folder, &line, &mut last_notable_message);
        }

        let outcome = if let Err(()) = drain_result {
            let _ = child.kill();
            Err(format!(
                "timed out waiting for adbsync output while syncing \"{folder}\" \
                 (no output for {SYNC_INACTIVITY_TIMEOUT:?})"
            ))
        } else {
            determine_folder_outcome(
                drain_result.ok().flatten(),
                last_notable_message.take(),
                &stderr_tail,
                subcommand,
            )
        };

        match &outcome {
            Ok(()) => {
                let _ = self
                    .app
                    .emit("sync-folder-success", FolderSuccessPayload { folder });
            }
            Err(message) => {
                let _ = self.app.emit(
                    "sync-folder-failure",
                    FolderFailurePayload {
                        folder,
                        error: message,
                    },
                );
            }
        }

        outcome
    }

    /// Parse one line and, if it carries a [`ProgressEvent`], emit it and
    /// remember it as the most recent "notable" (fatal/error) message so
    /// far, so a failure can be reported with the real adbsync/adb error
    /// text instead of a generic message.
    fn handle_line(
        app: &tauri::AppHandle,
        folder: &str,
        line: &str,
        last_notable_message: &mut Option<String>,
    ) {
        let Some(event) = progress_parser::parse_line(line) else {
            return;
        };
        match &event {
            ProgressEvent::Fatal { message } | ProgressEvent::Error { message } => {
                *last_notable_message = Some(message.clone());
            }
            ProgressEvent::Copying { .. } => {}
        }
        let _ = app.emit("sync-progress", SyncProgressPayload { folder, event });
    }
}

impl FolderSyncRunner for RealFolderSyncRunner {
    fn run_one_folder(&mut self, folder: &str) -> Result<(), String> {
        // `run_one_folder` is deliberately synchronous (see the trait doc
        // comment) so that `run_batch` stays a plain, easily-testable
        // function. The real work needs the shell plugin's async event
        // channel, though, so bridge the two with `block_on`. This is only
        // safe because every caller of `run_one_folder` on this concrete
        // type runs inside a `spawn_blocking` task (see `run_sync_batch`
        // below) -- a dedicated blocking-pool thread, not a tokio worker
        // thread executing an async task -- so nesting a blocking
        // `Runtime::block_on` here does not deadlock or panic the way it
        // would from directly inside an `async fn` Tauri command.
        tauri::async_runtime::block_on(self.run_one_folder_async(folder))
    }
}

/// Shared implementation behind the `run_backup`/`run_restore` Tauri
/// commands: build the real runner, run the batch on a blocking task (see
/// the note on `FolderSyncRunner for RealFolderSyncRunner` above), and emit
/// the final batch-outcome event.
async fn run_sync_batch(
    app: tauri::AppHandle,
    serial: String,
    dest: String,
    included_paths: Vec<String>,
    direction: SyncDirection,
) -> Result<BatchOutcome, String> {
    // Reject a malformed included path (relative, or containing a `..`
    // segment) before it's ever used to build a local destination path or
    // passed to `adb`/`adbsync` -- see `path_pair::validate_android_path`'s
    // doc comment for why `leaf_name` alone isn't a sufficient guard.
    path_pair::validate_included_paths(&included_paths)?;

    let mut runner = RealFolderSyncRunner::new(app.clone(), serial, dest, direction)?;

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let folder_refs: Vec<&str> = included_paths.iter().map(String::as_str).collect();
        run_batch(&folder_refs, &mut runner)
    })
    .await
    .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "sync-batch-complete",
        BatchCompletePayload {
            outcome: outcome.clone(),
        },
    );

    Ok(outcome)
}

/// Backup: pull the included paths from the device at `serial` into `dest`.
#[tauri::command]
pub async fn run_backup(
    app: tauri::AppHandle,
    serial: String,
    dest: String,
    included_paths: Vec<String>,
) -> Result<BatchOutcome, String> {
    run_sync_batch(app, serial, dest, included_paths, SyncDirection::Backup).await
}

/// Restore: push the included paths from `dest` back onto the device at
/// `serial`. Same "one call per included leaf path" primitive as backup,
/// source/dest swapped -- see design doc §3.
#[tauri::command]
pub async fn run_restore(
    app: tauri::AppHandle,
    serial: String,
    dest: String,
    included_paths: Vec<String>,
) -> Result<BatchOutcome, String> {
    run_sync_batch(app, serial, dest, included_paths, SyncDirection::Restore).await
}

#[cfg(test)]
mod orchestration_tests {
    use super::*;

    struct FakeRunner {
        results: Vec<Result<(), String>>,
    }

    impl FolderSyncRunner for FakeRunner {
        fn run_one_folder(&mut self, _folder: &str) -> Result<(), String> {
            self.results.remove(0)
        }
    }

    #[test]
    fn stops_the_batch_on_first_failure_and_reports_which_folder() {
        let mut runner = FakeRunner {
            results: vec![Ok(()), Err("disk full".into()), Ok(())],
        };
        let folders = vec!["DCIM", "Pictures", "Movies"];
        let outcome = run_batch(&folders, &mut runner);

        assert_eq!(outcome.completed, vec!["DCIM"]);
        assert_eq!(
            outcome.failed_at,
            Some(("Pictures".to_string(), "disk full".to_string()))
        );
        // Movies never attempted -- FakeRunner would panic on an empty
        // `results` vec if it were.
    }

    #[test]
    fn all_succeed_marks_the_whole_batch_complete() {
        let mut runner = FakeRunner {
            results: vec![Ok(()), Ok(())],
        };
        let folders = vec!["DCIM", "Pictures"];
        let outcome = run_batch(&folders, &mut runner);

        assert_eq!(outcome.completed, vec!["DCIM", "Pictures"]);
        assert_eq!(outcome.failed_at, None);
    }

    #[test]
    fn empty_batch_completes_with_nothing_done() {
        let mut runner = FakeRunner { results: vec![] };
        let outcome = run_batch(&[], &mut runner);

        assert_eq!(outcome.completed, Vec::<String>::new());
        assert_eq!(outcome.failed_at, None);
    }

    #[test]
    fn leaf_name_takes_the_last_path_segment() {
        assert_eq!(leaf_name("/storage/emulated/0/DCIM"), "DCIM");
        assert_eq!(leaf_name("/storage/emulated/0/DCIM/"), "DCIM");
        assert_eq!(leaf_name("DCIM"), "DCIM");
    }

    #[test]
    fn zero_exit_with_no_notable_message_is_a_success() {
        let outcome = determine_folder_outcome(Some(Some(0)), None, "", "pull");
        assert_eq!(outcome, Ok(()));
    }

    #[test]
    fn zero_exit_but_an_observed_error_event_is_still_reported_as_a_failure() {
        // The core regression this locks in: `progress_parser::parse_line`
        // can emit a non-fatal `ProgressEvent::Error` (e.g. a bare, unwrapped
        // `ls: ...: Permission denied` line -- see that module's doc
        // comment) for a process that still happens to exit 0. That must
        // NOT be reported as `sync-folder-success`.
        let outcome = determine_folder_outcome(
            Some(Some(0)),
            Some("ls: /sdcard/Android/data/foo: Permission denied".to_string()),
            "",
            "pull",
        );
        assert_eq!(
            outcome,
            Err("ls: /sdcard/Android/data/foo: Permission denied".to_string())
        );
    }

    #[test]
    fn zero_exit_with_unrecognized_stderr_text_is_still_reported_as_a_failure() {
        // The critical regression this locks in: real disk-full/IO-error
        // text like "No space left on device" is NOT matched by anything in
        // `progress_parser.rs` (unlike the `[CRITICAL] ...`/`ls: ... :
        // Permission denied` patterns covered by the test above), so
        // `last_notable_message` stays `None` for it. Before this fix, that
        // meant a zero exit code alone was enough to report the folder as
        // `sync-folder-success` even though the transfer actually failed --
        // exactly the failure mode the whole feature exists to prevent
        // (design doc §5). Mirrors `space.rs`'s
        // `exit_zero_with_stderr_output_is_treated_as_a_failure` test for
        // `interpret_du_exit`.
        let outcome = determine_folder_outcome(
            Some(Some(0)),
            None,
            "No space left on device",
            "push",
        );
        assert!(outcome.is_err());
        let message = outcome.unwrap_err();
        assert!(message.contains("No space left on device"));
    }

    #[test]
    fn zero_exit_with_only_benign_info_log_lines_is_a_success() {
        // Real regression #1, found via manual QA: `adbsync`'s own logging
        // setup (`SAOLogging.py::setup_root_logger`) attaches a
        // `logging.StreamHandler()` -- which defaults to stderr -- at level
        // INFO by default, so a completely successful transfer still logs
        // its tree-diff diagnostics to stderr on every run. This is the
        // exact shape of real output captured during manual testing
        // (`SGCAM` folder).
        let stderr = "[INFO] Source tree:\n\
                       [INFO] \u{251c}.: (1713617100, 1713617100)\n\
                       [INFO] \u{2514}8.5.300\n\
                       [INFO] \u{251c}.: (1713879720, 1713879720)\n\
                       [INFO] \u{2514}LOGCAT\n";
        let outcome = determine_folder_outcome(Some(Some(0)), None, stderr, "pull");
        assert_eq!(outcome, Ok(()));
    }

    #[test]
    fn zero_exit_with_adbs_own_native_progress_summary_lines_is_a_success() {
        // Real regression #2, found via manual QA immediately after fixing
        // #1: `adb`'s own "N file(s) pulled/pushed" progress-summary lines
        // (e.g. `.../Invisible .ogg: 1 file pulled, 0 skipped. 37.6 MB/s
        // (1645677 bytes in 0.042s)`) go straight to stderr with NO
        // `[LEVEL]` prefix at all -- `adbsync` runs `adb pull`/`push` with
        // inherited, uncaptured stdio in `--show-progress` mode (confirmed
        // in `docs/research/adbsync-tooling.md`), so this text never goes
        // through Python's logging module. An `[INFO]`/`[DEBUG]`-prefix
        // allowlist missed this entirely, which is exactly why this moved
        // to a denylist of specific known-bad text instead of trying to
        // enumerate every benign shape. This is the real `Ringtones`
        // folder's captured output (successful transfer, 3 files pulled).
        let stderr = "[INFO] SYNCING\n\
                       [INFO] Copying copy tree\n\
                       /storage/emulated/0/Ringtones/Compositions/Invisible .ogg: \
                       1 file pulled, 0 skipped. 37.6 MB/s (1645677 bytes in 0.042s)\n\
                       /storage/emulated/0/Ringtones/Compositions/SAMPHA.ogg: \
                       1 file pulled, 0 skipped. 17.9 MB/s (127029 bytes in 0.007s)\n";
        let outcome = determine_folder_outcome(Some(Some(0)), None, stderr, "pull");
        assert_eq!(outcome, Ok(()));
    }

    #[test]
    fn zero_exit_with_documented_disk_full_text_is_still_a_failure() {
        // The denylist's actual job: the two real, documented failure
        // phrases from `docs/research/` must still be caught even though
        // arbitrary other stderr content no longer is.
        let outcome = determine_folder_outcome(
            Some(Some(0)),
            None,
            "adb: error: cannot write '/sdcard/foo.jpg': Input/output error",
            "pull",
        );
        assert!(outcome.is_err());

        let outcome = determine_folder_outcome(
            Some(Some(0)),
            None,
            "No space left on device",
            "push",
        );
        assert!(outcome.is_err());
    }

    #[test]
    fn stderr_contains_known_fatal_text_is_case_insensitive_and_narrow() {
        assert!(stderr_contains_known_fatal_text("No Space Left on device"));
        assert!(stderr_contains_known_fatal_text(
            "adb: error: cannot write 'x': Input/Output Error"
        ));
        assert!(!stderr_contains_known_fatal_text(
            "[INFO] a\n[DEBUG] b\n[WARNING] c\n[ERROR] d\n[CRITICAL] e\n"
        ));
        assert!(!stderr_contains_known_fatal_text(""));
        assert!(!stderr_contains_known_fatal_text(
            "1 file pulled, 0 skipped. 37.6 MB/s (1645677 bytes in 0.042s)"
        ));
    }

    #[test]
    fn nonzero_exit_with_a_notable_message_uses_that_message() {
        let outcome = determine_folder_outcome(
            Some(Some(1)),
            Some("adbsync: fatal error".to_string()),
            "some stderr noise",
            "pull",
        );
        assert_eq!(outcome, Err("adbsync: fatal error".to_string()));
    }

    #[test]
    fn nonzero_exit_with_no_notable_message_falls_back_to_a_generic_message() {
        let outcome = determine_folder_outcome(Some(Some(1)), None, "boom", "push");
        assert_eq!(
            outcome,
            Err("adbsync push exited with code Some(1): boom".to_string())
        );
    }

    #[test]
    fn process_terminated_without_an_exit_code_is_a_failure() {
        let outcome = determine_folder_outcome(Some(None), None, "killed", "pull");
        assert_eq!(
            outcome,
            Err("adbsync pull exited with code None: killed".to_string())
        );
    }

    #[test]
    fn channel_closed_without_ever_seeing_terminated_is_a_failure() {
        let outcome = determine_folder_outcome(None, None, "eof", "pull");
        assert_eq!(
            outcome,
            Err("adbsync pull process ended unexpectedly: eof".to_string())
        );
    }

    #[test]
    fn line_splitter_splits_on_lf_and_bare_cr_and_buffers_partial_lines() {
        let mut splitter = LineSplitter::default();

        // A chunk boundary landing mid-line shouldn't produce a line until
        // the rest arrives.
        assert_eq!(splitter.push("[ 10%] IMG"), Vec::<String>::new());
        assert_eq!(
            splitter.push("_0001.jpg\r[ 55%] IMG_0001.jpg\r"),
            vec!["[ 10%] IMG_0001.jpg", "[ 55%] IMG_0001.jpg"]
        );
        assert_eq!(
            splitter.push("[100%] IMG_0001.jpg\n"),
            vec!["[100%] IMG_0001.jpg"]
        );
        assert_eq!(splitter.take_remainder(), None);
    }

    #[test]
    fn line_splitter_flushes_a_trailing_partial_line_on_stream_close() {
        let mut splitter = LineSplitter::default();
        assert_eq!(splitter.push("[CRITICAL] Exiting"), Vec::<String>::new());
        assert_eq!(
            splitter.take_remainder(),
            Some("[CRITICAL] Exiting".to_string())
        );
        // Draining twice in a row shouldn't resurrect the same line.
        assert_eq!(splitter.take_remainder(), None);
    }
}
