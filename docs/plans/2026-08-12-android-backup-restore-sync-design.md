# Android Backup/Restore Design

**Status:** Approved design, not yet implemented.

**Input:** `docs/research/` (domain knowledge, `adbsync` tooling findings, Windows
gotchas, the disk-full/nested-duplicate incident, and the requirements they imply)
— written up after a manual `adbsync`/PowerShell backup effort. This design
directly answers `docs/research/app-requirements.md`'s open questions.

## Scope (v1)

- **Backup + Restore**, both directions (phone → PC and PC → phone). Not
  continuous/bidirectional sync — one-shot runs in either direction, initiated
  by the user.
- **Windows only.** Matches everything gathered in research (Windows
  platform-tools, Windows-specific gotchas already researched, dev machine).
  Cross-platform is deferred, not designed against here.
- **Multiple devices**, each with a saved profile (classification + destination),
  keyed by ADB serial.
- Hash-based integrity verification is **on-demand only** (an explicit "verify"
  action), not automatic on every run — full-tree hashing a 100+GB media library
  every run is expensive and wasn't a v1 requirement in the research.

## 1. Architecture

**Build-time:** `better-adb-sync` (GitHub: `jb2170/better-adb-sync`, PyPI:
`BetterADBSync`, CLI command `adbsync`) is vendored as a git submodule pinned to
release tag `v1.4.0`. A build script freezes it into a standalone `adbsync.exe`
via PyInstaller. That, plus `adb.exe`/`fastboot.exe`/DLLs from
`platform-tools-latest-windows.zip`, are bundled as Tauri sidecar binaries —
nothing for the user to install separately.

Verified directly against the vendored source (not just prior research notes):
- Zero third-party pip dependencies (stdlib only) — freezes cleanly, small
  build, no surprise dependency issues.
- Apache 2.0 licensed — permissive enough to vendor and redistribute a frozen
  build (needs attribution/NOTICE).
- Confirmed entry point: `pyproject.toml` → `adbsync = "BetterADBSync:main"`.

**Runtime:** the Tauri (Rust) backend never talks to the phone directly — it
shells out to the bundled `adbsync.exe`, always passing `--adb-bin` pointing at
the bundled `adb.exe` (never relies on ambient PATH). One `adbsync pull`/`push`
subprocess call per included top-level folder — never a whole-tree call with
excludes (see Error Handling; the crash mechanism is confirmed at the source
level). Rust owns constructing every `ANDROID`/`LOCAL` path pair itself, always
with the correct trailing slash — the UI never lets the user type a raw path
pair, closing off the nesting-duplicate bug at the source
(`BetterADBSync/__init__.py:323`, confirmed).

**Frontend:** React + astryxdesign (already installed — see `.claude/CLAUDE.md`
for its conventions: no raw `<div>` layout, `AppShell` as root, tokens-only
styling, discover components via `astryx build`/`astryx component` rather than
freehanding). Drives device selection, per-folder classification review, and
progress — calling Tauri commands that wrap the subprocess calls.

**Accepted trade-off:** still shelling out to a subprocess rather than a native
Rust ADB implementation, so `adbsync`'s mtime-only diffing (rounded to the
minute, no hash/size check — confirmed at `BetterADBSync/FileSystems/Base.py:39`)
is inherited as-is for v1. This is why on-demand hash verification exists as a
separate, explicit action rather than being silently trusted.

## 2. Components

**Rust backend (Tauri commands):**
- `devices::list()` — runs bundled `adb.exe devices -l`, returns serials +
  display names.
- `profile::load_or_create(serial)` — reads/writes a per-device profile row
  (destination path) plus its `folder_rules` (see Data Storage), keyed by ADB
  serial.
- `classify::suggest(serial)` — runs `adb shell ls` on the profile root, applies
  the heuristics from `docs/research/android-storage-domain.md` (personal
  media → include, stock/empty → skip, stale duplicate of
  `Android/media/<pkg>` → skip + flag, `Android/data`/`Android/obb` → always
  excluded from any walk), returns a suggested include/skip list for the
  frontend to render as pre-checked toggles.
- `space::check(dest, estimated_bytes)` — free-space preflight, plus a
  cloud-sync-path detector (OneDrive/Dropbox/etc. path pattern match) for the
  warning.
- `sync::run_backup(serial, dest, included_paths)` /
  `sync::run_restore(serial, source, included_paths)` — for each included leaf
  path, builds the `ANDROID`/`LOCAL` pair internally (always correct trailing
  slash), spawns bundled `adbsync.exe` sequentially per folder, parses its
  `--show-progress` output, emits Tauri progress events per folder/file, and
  writes `runs`/`run_items` rows (see Data Storage).

**React frontend (astryxdesign components):**
- **Device screen** — list connected devices, pick one, shows whether it's a
  known profile or new.
- **Classification screen** — shown for a new device (or on demand for
  review): folder list with heuristic-suggested toggles, explicitly flags the
  stale-duplicate case (e.g. root `WhatsApp` vs.
  `Android/media/com.whatsapp/WhatsApp`) rather than silently picking one.
- **Run screen** (shared shape for backup and restore) — preflight results
  (free space, cloud-sync warning) gate the start button; per-folder progress
  once running.
- **History view** — run list (from `runs`/`run_items`): last synced time per
  path, outdated/broken/not-synced status, per past run.
- **Profile settings** — list of saved device profiles, edit
  destination/classification for any of them.

## 3. Data Flow

**First run for a device:**
1. App launches → lists connected devices via bundled `adb.exe`.
2. User picks a device → no saved profile found → `classify::suggest` runs
   `adb shell ls` against the storage root, applies heuristics, returns a
   suggested list.
3. User reviews/edits the suggested list on the Classification screen, sets a
   destination folder.
4. Profile (`devices` row + `folder_rules` rows) saved.

**Backup run:**
1. User hits Backup → preflight (`space::check`): free space vs. rough
   estimate, cloud-sync-path warning. Blocks start until resolved or
   explicitly acknowledged.
2. `sync::run_backup` creates a `runs` row (`type = backup`, `status =
   running`), then iterates the included paths one at a time (never a
   whole-tree call), spawning `adbsync.exe pull` per folder with the
   correctly-built trailing-slash pair.
3. Each subprocess's `--show-progress` output is parsed and emitted as Tauri
   events → frontend updates a per-folder/per-file progress view. A
   `run_items` row is written per path on completion (status, bytes, file
   count, error if any).
4. On completion of all folders, `runs.status = completed`,
   `runs.finished_at` set. Re-running later is the same flow — `adbsync`'s
   mtime diffing makes it incremental automatically.

**Restore run:** same shape in reverse — `sync::run_restore` pushes from the
local destination back to the phone, reusing the same profile's path mapping.
Push/pull share the same "one call per included leaf path, app builds the
pair" primitive, so this is the same code path with source/dest swapped, not a
separate implementation.

**Returning to an existing device:** skip classification (profile already
exists), go straight to the Run screen — user can still open Profile Settings
to edit classification/destination if something changed.

## 4. Data Storage

SQLite via the Tauri SQL plugin, with Drizzle ORM using Drizzle's SQLite
proxy-driver pattern (schema/queries in TypeScript, executed through the Tauri
plugin's `rusqlite`-backed connection over IPC).

**Schema:**
- `devices` — `serial` (PK), `display_name`, `first_seen`, `last_seen`
- `folder_rules` — `device_serial` (FK), `path`, `decision` (include/skip),
  `source` (heuristic/manual), `updated_at`
- `runs` — `id` (PK), `device_serial` (FK), `type` (backup/restore),
  `started_at`, `finished_at`, `status` (running/completed/failed/cancelled)
- `run_items` — `run_id` (FK), `path`, `status`
  (synced/outdated/broken/skipped/error), `bytes_transferred`, `file_count`,
  `error_message`, `finished_at`

**Deriving displayed status:**
- **Last synced** — `MAX(finished_at)` from `run_items` for a given path.
- **History** — `runs`/`run_items` directly; a run list view is just a query.
- **Not-synced** — a path in `folder_rules` with `decision = include` and no
  matching `run_items` row.
  _Implementation note (Task 14, `history-screen.tsx`):_ implemented as no
  matching `run_items` row with `status = synced`, not literally "no matching
  row of any status" — an `error` row's `finished_at` marks a failed attempt,
  not a successful sync, so counting it would misreport a broken path as
  freshly synced. Same underlying concept, refined at implementation time.
- **Outdated** — `adbsync` itself re-transfers a changed file at run time; we
  record `outdated → synced` for that path in that run's `run_items`, so "was
  this outdated last run" is visible in history without re-deriving it.
- **Broken** — a subprocess/file-level error recorded in
  `run_items.error_message` (nonzero exit, I/O error, etc.) — not a hash
  check. Content hashing is a separate, on-demand "verify" action (see Scope),
  not automatic.

## 5. Error Handling

**Preflight (before any transfer starts):**
- Bundled `adb.exe`/`adbsync.exe` presence check — defensive only (packaging
  bug if missing, not a user-facing setup step).
- Device authorized and reachable (`adb devices -l` shows `device`, not
  `unauthorized`/`offline`).
- Free space vs. rough estimate (`adb shell du` over included folders,
  best-effort, labeled as an estimate in the UI).
- Cloud-sync destination path detected → warning, not a hard block.

**Mid-run:**
- Each `adbsync.exe` call is per-folder, so a failure is scoped to one
  `run_items` row, not the whole run — the batch stops there, records
  `status = failed` with the actual stderr/exit text as `error_message`
  (never a generic message), and leaves already-completed folders `synced`.
- No blind retry loop. The disk-full "looks like it's re-copying everything
  forever" symptom from the research (`docs/research/incident-disk-full-nested-duplicates.md`)
  only happens when nothing surfaces the real cause — surfacing the actual
  adb/adbsync error text directly addresses that.
- Free space is re-checked between folders, not just once upfront.

**Resuming after a failed/partial run:** just re-run — `adbsync`'s mtime
diffing plus `run_items` history means it's safe and picks up where it left
off.

**Local cleanup (if ever needed):** confirm-before-delete only — check against
the device via `adb shell ls` before deleting anything locally, per
`docs/research/incident-disk-full-nested-duplicates.md`'s diagnostic method
(structural anomaly → size check → verify against device → only then delete).
Never automatic deletion based on local heuristics alone.

## 6. Testing

- **Rust unit tests (pure logic, no device needed):**
  - `ANDROID`/`LOCAL` path-pair builder — given a folder path and destination,
    always produces the correct trailing-slash form. The single most
    important test in the plan, since it's the fix for the confirmed nesting
    bug (`BetterADBSync/__init__.py:323`).
  - Classification heuristic — given a canned folder listing (name, size,
    mtime), produces the expected include/skip/flag-as-stale-duplicate
    suggestions, using the test-device table from
    `docs/research/android-storage-domain.md` as fixture data.
  - Free-space check math.
- **Subprocess output parsing tests:** feed canned `adbsync --show-progress`
  stdout/stderr (including a canned permission-denied line) through the
  parser, assert the right progress events/errors come out, and that no code
  path ever attempts a whole-tree call.
- **PyInstaller build smoke test:** after freezing `adbsync.exe` from the
  pinned `v1.4.0` submodule, run `adbsync.exe --version`/`--help` in CI to
  catch a broken freeze before it ships.
- **Manual/E2E checklist (not automated):** real device backup + restore round
  trip, disk-full behavior, cloud-sync-destination warning trigger — these
  need real hardware and can't run in CI.

## Open items carried forward (not resolved by this design)

From `docs/research/app-requirements.md`, still unresolved:
- Whether secondary Android profiles (Private Space, Dual Apps) are readable
  over ADB at all.
- Whether a native Rust ADB invocation path (if ever pursued instead of
  shelling out) reintroduces any Windows path-mangling equivalent — N/A for
  this design since it shells out, but worth remembering if that decision is
  revisited.

## Next Step

Hand off to `superpowers:writing-plans` to produce a task-by-task
implementation plan from this design.
