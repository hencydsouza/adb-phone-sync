# Windows / Environment Gotchas

Encountered while driving `adb`/`adbsync` manually from Git Bash on Windows.
Flagged per-item whether it's expected to persist in this app (a native
Tauri/Rust implementation) or was specific to the shell-based workaround.

## Git Bash (MSYS) path mangling

`adb shell` commands using unix-style paths (`/storage/...`) get mangled by
MSYS's automatic path conversion into bogus Windows paths when run from Git
Bash. Same problem affects `cmd.exe /c "..."` invoked from Git Bash — the
`/c` flag itself gets path-converted.

- **Workaround used:** prefix commands with `MSYS_NO_PATHCONV=1`.
- **Relevance to this app:** likely N/A — a Rust `Command`/`adb` invocation
  from the Tauri backend doesn't go through MSYS/Git-Bash, so this class of
  bug shouldn't reoccur. Worth a smoke test once the Rust ADB invocation path
  exists, to confirm no equivalent path-mangling happens via `std::process`
  or whatever ADB client library gets used.

## OneDrive-synced destination folders

Using a backup destination inside a OneDrive-synced folder caused two
problems:

1. OneDrive tries to hydrate/sync every file as it lands, which can slow a
   large transfer significantly.
2. Confusing errors on plain reads — e.g. `ls -la` on a OneDrive folder
   surfaced `No space left on device` for a pure read, apparently because
   listing cloud-placeholder files triggered a hydration attempt that itself
   needed disk space.

- **Relevance to this app:** directly relevant — see
  [app-requirements.md](./app-requirements.md). The app should default the
  destination picker away from cloud-synced folders, or detect and warn when
  the user picks one (OneDrive, Dropbox, Google Drive, etc.).

## Disk space exhaustion symptom

Not Windows-specific, but surfaced through Windows tooling: when the
destination drive fills completely mid-transfer, every subsequent write
fails (`adb: error: cannot write '...': Input/output error`), and re-running
the same command afterward *looks* like it's re-copying every file from
scratch. It isn't re-copying blindly — nothing ever finished writing the
first time, so every comparison keeps seeing the destination file as
missing/incomplete. See
[incident-disk-full-nested-duplicates.md](./incident-disk-full-nested-duplicates.md)
for the full incident this caused, compounded with a second, unrelated bug.
