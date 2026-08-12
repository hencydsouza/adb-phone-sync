//! Parses individual stdout lines produced by `adbsync --show-progress`
//! (and the `adb push`/`adb pull` processes it shells out to) into
//! structured [`ProgressEvent`]s.
//!
//! The exact line formats implemented here were reconciled against the
//! vendored `better-adb-sync` v1.4.0 source
//! (`third_party/better-adb-sync/src/BetterADBSync/`) and against string
//! constants extracted from the frozen `adb.exe` binary
//! (`src-tauri/binaries/adb-x86_64-pc-windows-msvc.exe`), rather than
//! trusted from the implementation plan's guess. See doc comments below on
//! each branch of [`parse_line`] for what was verified and where.

#[derive(Debug, PartialEq, Eq, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum ProgressEvent {
    /// A file is actively being transferred. `path` is the device-side path
    /// (source for a pull, destination for a push) as printed by `adb`
    /// itself in its `"[%3d%%] %s"` progress line.
    Copying { path: String },
    /// adbsync hit an unrecoverable condition and is about to exit
    /// (`logging_fatal` / `logging.critical` in the Python source).
    Fatal { message: String },
    /// A non-fatal but noteworthy error line that must be surfaced to the
    /// user rather than silently dropped (e.g. a `Permission denied` line
    /// that leaked through unwrapped).
    Error { message: String },
}

/// Parse a single line of subprocess stdout. Returns `None` for blank lines
/// or any line that carries no information the caller needs to act on
/// (e.g. adb's own file-count summary lines, `pull: building file list...`,
/// tree-dump headers, etc.) -- those are intentionally ignored, not an
/// oversight.
pub fn parse_line(line: &str) -> Option<ProgressEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // adb's real progress display redraws in place using `\r` (not `\n`)
    // between percentage updates for the same file. If the caller's stream
    // splitter is `\n`-only (e.g. a naive `BufReader::lines()`), multiple
    // redraw updates for one file can arrive concatenated into a single
    // "line" containing embedded `\r`s, e.g.
    // `"[  1%] IMG.jpg\r[ 45%] IMG.jpg\r[100%] IMG.jpg"`. Only the text
    // after the LAST `\r` is the most recent update; keep that and discard
    // the stale prefix so it can never leak into a parsed field (e.g.
    // `path`). This is a no-op whenever `\r` isn't embedded, which covers
    // every real line we've seen so far.
    let line = line.rsplit('\r').next().unwrap_or(line).trim();
    if line.is_empty() {
        return None;
    }

    // VERIFIED: BetterADBSync/__init__.py:333-337 configures the root
    // logger with `messagefmt = "[%(levelname)s] %(message)s" if os.name ==
    // "nt" else "%(message)s"`. Our target platform is Windows, so a fatal
    // `logging.critical(...)` / `logging_fatal(...)` call (SAOLogging.py)
    // surfaces on stdout as exactly `[CRITICAL] <message>` -- no timestamp,
    // no "(file:line)" suffix (those only appear in SAOLogging.py's default
    // format string, which is overridden by the app's own `nt` format).
    // Confirmed call site: Android.py's `line_not_captured` logs
    // `logging.critical("ADB line not captured")` verbatim, matching the
    // plan's guessed test input exactly.
    if let Some(rest) = line.strip_prefix("[CRITICAL] ") {
        return Some(ProgressEvent::Fatal {
            message: rest.to_string(),
        });
    }

    // Defensive fallback for a bare, unprefixed `ls: ...: Permission
    // denied` line. In the confirmed real crash path (Bug 2, see
    // docs/research/adbsync-tooling.md and Android.py's
    // `line_not_captured` -> `logging_fatal(line)`), this exact text is
    // actually already wrapped in `[CRITICAL] ` on Windows and is caught by
    // the branch above instead. This branch exists purely so that if such a
    // line ever does reach us unwrapped (different platform build, a raw
    // `ls` stderr line surfacing outside the logging path, a future
    // adbsync version), it is still surfaced rather than silently dropped.
    if line.starts_with("ls:") && line.ends_with("Permission denied") {
        return Some(ProgressEvent::Error {
            message: line.to_string(),
        });
    }

    // VERIFIED: adbsync's own Python layer does NOT log a "Copying <path>"
    // line in --show-progress mode -- that string does not exist anywhere
    // in the vendored source. Base.py's `push_tree_here` only calls
    // `logging.info(relative_tree_path)` `if not show_progress`; when
    // show_progress is True that per-file log call is skipped entirely and
    // `push_file_here` (Local.py / Android.py) runs `adb push`/`adb pull`
    // with inherited stdio instead of redirecting it. So the real per-file
    // "in progress" text a caller sees comes straight from adb.exe itself.
    // Confirmed via ASCII strings extracted from the vendored
    // `adb-x86_64-pc-windows-msvc.exe`: the progress format constant is
    // `"[%3d%%] %s"`, i.e. lines like `[ 45%] /sdcard/DCIM/IMG_0001.jpg` or
    // `[100%] /sdcard/DCIM/IMG_0001.jpg`.
    if let Some(path) = parse_adb_progress_line(line) {
        return Some(ProgressEvent::Copying {
            path: path.to_string(),
        });
    }

    None
}

/// Matches adb's own `"[%3d%%] %s"` progress format, e.g.
/// `"[ 45%] /sdcard/DCIM/IMG_0001.jpg"`. Returns the path portion.
fn parse_adb_progress_line(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (percent, path) = rest.split_once("%] ")?;
    let percent = percent.trim();
    if percent.is_empty() || !percent.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if path.is_empty() {
        return None;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_file_copying_progress_line() {
        // Real `adb push`/`adb pull` per-file progress format, confirmed via
        // the `"[%3d%%] %s"` string constant in the vendored adb.exe binary.
        let events = parse_line("[ 45%] /storage/emulated/0/DCIM/Camera/IMG_0001.jpg");
        assert_eq!(
            events,
            Some(ProgressEvent::Copying {
                path: "/storage/emulated/0/DCIM/Camera/IMG_0001.jpg".into()
            })
        );
    }

    #[test]
    fn parses_a_completed_file_progress_line_at_100_percent() {
        let events = parse_line("[100%] /storage/emulated/0/DCIM/Camera/IMG_0001.jpg");
        assert_eq!(
            events,
            Some(ProgressEvent::Copying {
                path: "/storage/emulated/0/DCIM/Camera/IMG_0001.jpg".into()
            })
        );
    }

    #[test]
    fn parses_a_critical_error_line_as_a_fatal_event() {
        // Verbatim call site: Android.py's `line_not_captured` does
        // `logging.critical("ADB line not captured")`, which the app's
        // Windows log format (`"[%(levelname)s] %(message)s"`) renders as
        // exactly this string.
        let events = parse_line("[CRITICAL] ADB line not captured");
        assert_eq!(
            events,
            Some(ProgressEvent::Fatal {
                message: "ADB line not captured".into()
            })
        );
    }

    #[test]
    fn treats_the_real_permission_denied_crash_line_as_fatal_not_swallowed() {
        // In the real Bug 2 crash path, the raw `ls: ...: Permission denied`
        // text is logged via `logging.critical(line)` (SAOLogging.py's
        // `logging_fatal`), so on Windows it arrives already wrapped in
        // `[CRITICAL] `. It must still be surfaced, just as a Fatal rather
        // than a bare Error.
        let events = parse_line(
            "[CRITICAL] ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied",
        );
        assert_eq!(
            events,
            Some(ProgressEvent::Fatal {
                message: "ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied"
                    .into()
            })
        );
    }

    #[test]
    fn surfaces_a_bare_permission_denied_line_instead_of_swallowing_it() {
        // Defensive fallback: if this text ever reaches us unwrapped (no
        // `[CRITICAL] ` prefix), it must still be surfaced rather than
        // silently dropped.
        let events =
            parse_line("ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied");
        assert_eq!(
            events,
            Some(ProgressEvent::Error {
                message: "ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied"
                    .into()
            })
        );
    }

    #[test]
    fn embedded_carriage_returns_from_concatenated_redraws_keep_only_the_latest_update() {
        // adb redraws its progress line in place using `\r`, not `\n`,
        // between percentage updates for the same file. If a future `\n`-only
        // stream splitter ever concatenates several redraws into one "line",
        // this must still extract just the LAST (most recent) update rather
        // than corrupting `path` with the stale prefix.
        let events =
            parse_line("[  1%] IMG.jpg\r[ 45%] IMG.jpg\r[100%] IMG.jpg");
        assert_eq!(
            events,
            Some(ProgressEvent::Copying {
                path: "IMG.jpg".into()
            })
        );
    }

    #[test]
    fn ignores_a_bracket_line_with_a_non_numeric_percent() {
        assert_eq!(parse_line("[abc%] file"), None);
    }

    #[test]
    fn ignores_a_bracket_line_with_no_trailing_path() {
        assert_eq!(parse_line("[ 45%]"), None);
    }

    #[test]
    fn ignores_blank_lines() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   "), None);
    }

    #[test]
    fn ignores_unrelated_informational_lines() {
        // e.g. adb's own final per-file summary line and adbsync's
        // "building file list" chatter -- noise the caller doesn't act on.
        assert_eq!(
            parse_line(
                "/sdcard/DCIM/IMG_0001.jpg: 1 file pulled, 0 skipped. 12.3 MB/s (2345678 bytes in 0.182s)"
            ),
            None
        );
        assert_eq!(parse_line("pull: building file list..."), None);
    }
}
