# Manual QA Checklist: Android Backup/Restore Sync

Companion to `2026-08-12-android-backup-restore-sync-implementation.md` (Tasks
1-16). This is a **manual, human-with-a-real-device** checklist — nothing
here runs in CI. Everything in this feature was built and verified through
Tasks 1-15 via `cargo test`/`bun test`, standalone harnesses, and (for a
handful of Rust modules) one-off runs against a real connected phone. **No
one has ever clicked through the running Tauri app end-to-end** — that
verification gap, plus two other real gaps found while writing this
checklist, are called out below as blockers before the rest of the list is
even attemptable.

## 0. Known Limitations — read this before doing anything else

These are pre-existing gaps in the current branch, not new bugs to file. QA
cannot "just try the app" without working around them.

1. **There is no cross-screen navigation.** `src/app.tsx` mounts only
   `DeviceScreen`, unconditionally:
   ```tsx
   <AppShell contentPadding={4}>
     <DeviceScreen onDeviceSelected={setSelectedSerial} />
   </AppShell>
   ```
   `ClassificationScreen`, `RunScreen`, `HistoryScreen`, and
   `ProfileSettingsScreen` are fully built and individually testable, but
   **unreachable from the compiled app**. There is no router, no tab bar, no
   "Next" button that swaps screens. To QA any screen past `DeviceScreen`
   today, a developer must hand-edit `app.tsx` to mount that screen directly
   (optionally wiring in hardcoded `serial`/`dest`/`includedPaths` props) and
   rebuild. **This is a blocker for a real, unmodified click-through of the
   app and should be flagged to whoever picks up the next phase of work** —
   it is not something this QA pass can "route around" and still claim the
   app works for a real user.

2. **`folder_rules` is never written to.** Task 12's `ClassificationScreen`
   keeps `included`/`suggestions` in local React state only (see
   `src/screens/classification-screen.tsx` around the "Selections saved"
   banner) — there is no Tauri command that persists a selection into the
   `folder_rules` table, despite the table existing in the schema since Task
   4. Concretely this means:
   - `HistoryScreen`'s "Not synced" derivation (`deriveFolderSyncStatus` in
     `src/screens/history-screen.tsx`) has no real data to derive from on a
     freshly-classified device — the `folder_rules` join will always be
     empty in practice today.
   - There is no real path from "user classified their folders" to "a
     backup run picks up those folders." `RunScreen` takes `includedPaths`,
     `dest`, and `serial` as **props with local text-input fallbacks**
     (`src/screens/run-screen.tsx` lines ~391-403) — a QA tester driving a
     real backup run today has to manually type ANDROID paths (newline
     separated) and a destination path into `RunScreen`'s own fields, not
     flow them from `ClassificationScreen`. Item 1 below accounts for this.

3. **The SQL migration registration bug (fixed in `862d3e3`/`57a33a8`) was
   only proven against a standalone sqlx harness, never the real Tauri app.**
   For the entire span of Tasks 4, 13, 14, and 15, `tauri_plugin_sql`'s
   `Builder` was registered with no `.add_migrations(...)` call, so a real
   running app would have created zero tables and every Drizzle-backed query
   (run persistence, history view, profile settings) would have failed at
   runtime — while `cargo test`/unit tests all stayed green, because none of
   them go through the real plugin's `load` command. `862d3e3` fixed the
   registration and `57a33a8` added a regression test
   (`migrations().len()` vs. Drizzle's `_journal.json` entry count) so it
   can't silently drift again — but the fix itself was verified with "a
   standalone sqlx-based harness that reproduces the plugin's internal
   MigrationSource/Migrator flow," per the commit message, not the actual
   compiled app. **Item 1 below exists specifically to close this gap.**

## 1. Priority 0 — unblock QA (do this first, once)

- [ ] **Wire minimal navigation.** Hand-edit `src/app.tsx` (or add a
      throwaway router) so `DeviceScreen -> ClassificationScreen ->
      RunScreen -> HistoryScreen` and `ProfileSettingsScreen` are all
      reachable in one build, with the selected serial/dest/paths actually
      flowing between them. This does not need to be production-quality —
      it needs to exist long enough to run the rest of this checklist
      against the real app shell. Note in the QA report that real routing
      is still a follow-up task.
- [ ] **Confirm the app boots with a fresh (deleted) sqlite db file and the
      migration fix actually creates tables.** Delete/rename the app's
      sqlite db file (`adb-phone-sync.db`, wherever Tauri's app-data dir
      resolves to on this machine), launch the real compiled app (not a
      test harness), and confirm: no startup error, a `devices` row can be
      written and read back (e.g. by completing the Device screen flow),
      and `history`/`profile settings` screens load without a "no such
      table" error. This is the first time this fix will have been checked
      against the real running app rather than a standalone harness — treat
      any failure here as a P0 bug, not a QA nit.

## 2. Priority 1 — real device round trip

Do these only after Section 1 passes. Use a real Android phone with USB
debugging enabled and a nontrivial DCIM/Pictures/etc. folder structure.

- [ ] **Full backup round trip.** Device screen → select device → (manually
      re-enter or navigate through) Classification screen → include a
      handful of real folders → Run screen → run a real backup to a local
      (non-cloud-synced) destination. Confirm: progress UI updates live,
      run completes, exit state is "success," and the files actually landed
      on disk with correct contents (spot check a few files by hash or
      size against the device).
- [ ] **Full restore round trip.** From the same (or a fresh) destination,
      run a restore back to the device (or a second test device/emulator
      if available) and confirm files land in the expected on-device paths
      and are intact.
- [ ] **Re-run a just-completed backup and confirm it's fast/incremental.**
      Immediately re-run the same backup with no source changes. Confirm it
      finishes in seconds (mtime diffing skipping already-copied files), not
      by re-copying everything. This directly re-validates the mtime-based
      incremental behavior implicated in the disk-full incident (see
      Section 3).
- [ ] **Nested-duplicate-folder regression check.** After the backup run
      above, inspect the destination tree structure directly (e.g.
      `find -maxdepth 2` per
      `docs/research/incident-disk-full-nested-duplicates.md`'s own
      diagnostic method). Confirm there is **no** `DCIM\DCIM`,
      `WhatsApp\WhatsApp`-style doubled nesting. This is a structural
      regression check for the original incident's root cause (missing
      trailing slash on ANDROID source paths) — the path-pair builder
      (`src-tauri/src/*path*` from Task 5) was built specifically to make
      this invariant impossible, but it has never been checked against a
      real multi-folder run end-to-end, only unit tests.

## 3. Priority 1 — failure-mode QA (the actual point of this feature)

- [ ] **Disk-full mid-run behavior.** Point a backup at a destination volume
      with only a few hundred MB free (smaller than the estimated transfer
      size — the preflight space check should already warn about this, see
      below, but force the run anyway if the UI allows it, or shrink free
      space mid-run) and confirm the app **stops cleanly with a clear
      error**, not the original incident's "silently retries the same file
      forever" symptom. Specifically check:
      - Does the run screen surface a distinct failure state (not "success"
        and not a silent hang)?
      - Is the error message something a non-technical user could act on
        (vs. a raw adb/Python traceback)?
      - **This has never been tested against a real disk-full condition.**
        `src-tauri/src/sync/orchestration.rs`'s "never report success when
        an Error/Fatal event was observed" guarantee (`3cef699`) and the
        progress parser's CRITICAL/Fatal classification (`ac6c9f8`) were
        both verified with synthetic strings and unit tests (e.g. the
        `"disk full".into()` fixture in `orchestration.rs`'s tests) — not
        against `adb`'s actual `Input/output error` / `No space left on
        device` output on a real full disk. There is no string match for
        `Input/output error` or `No space left on device` anywhere in
        `progress_parser.rs`. Note this was NOT actually covered by
        `8877362` (that commit's "any stderr = failure on exit 0" hardening
        only ever touched `space.rs`'s preflight `du -s` size estimate, not
        this actual transfer path — a final whole-tree review caught that
        this exact fix had never been ported over to `orchestration.rs`).
        As of `246d267`, `determine_folder_outcome` in `orchestration.rs`
        now applies the same "any non-empty stderr on a zero exit code is a
        failure" rule directly (mirroring `space.rs`'s `interpret_du_exit`),
        so real disk-full/IO-error text landing on stderr with a zero exit
        code is no longer silently reported as `sync-folder-success` even
        when `progress_parser.rs` doesn't recognize the specific wording.
        That rule has unit test coverage (a literal `"No space left on
        device"` fixture, verified to fail against the pre-fix logic) but,
        like the `du -s` case below, zero real-device verification — this
        item is still genuinely unverified end-to-end; budget real time for
        it and file a bug if real disk-full behavior doesn't match.
- [ ] **Cloud-sync destination warning.** Point a backup destination at a
      path inside a real OneDrive-synced folder. Confirm the "Destination is
      inside a cloud-synced folder" banner (`run-screen.tsx`, the
      `is_cloud_synced` check) actually appears, and that dismissing it
      (`isDismissable`/`onDismiss`) makes it go away and the run can proceed.
      Note from `5097ca0`'s review: `is_cloud_synced_path` does raw
      substring matching with no path canonicalization — try a path with
      mixed slashes or a relative segment (`..`) to see if the warning is
      fooled either direction (false negative worse than false positive).
- [ ] **Permission-restricted subtree during space estimate.** Include a
      folder likely to contain an inaccessible subfolder on a real device
      (e.g. `Android/data` or `Android/obb`) in the preflight `du -s`
      estimate and confirm the app fails the estimate clearly rather than
      silently under-counting. Per `8877362`'s commit message this defensive
      "stderr-even-with-exit-0 is a failure" behavior was added specifically
      for this case but explicitly **"can't be live-tested without a real
      permission-restricted device path"** — it has unit test coverage
      (simulated) but zero real-device verification. This is the single
      most concrete "verify this for real" item in the whole checklist.

## 4. Priority 2 — screen-by-screen spot checks (never GUI-clicked before)

None of the below have been exercised in the actual compiled app by anyone
during Tasks 11-15 — only via component-level tests or headless checks. Spot
check each for basic usability now that Section 1 makes them reachable.

- [ ] **Device screen.** Real device list populates via `adb`, selecting a
      device works, and the async-state/lifted-selection fixes from
      `e07f90c` hold up (rapidly switching devices doesn't leave stale
      state).
- [ ] **Device screen, disconnected state.** Unplug/authorize-revoke the
      device and reload — confirm the "no devices connected" `EmptyState`
      renders correctly (this was an explicit manual-verification step in
      the original Task 11 plan that never got exercised against the real
      compiled app).
- [ ] **Classification screen.** Suggestions come from the real
      `classify_suggest` command (already verified against a real device's
      `ls -la` output per `41c2f6a` — no need to re-verify the parsing
      itself, just that the screen renders it correctly). Toggling
      checkboxes clears the (currently non-persisting, see Section 0 item 2)
      "Selections saved" banner per `b8fff57`.
- [ ] **Run screen — direction toggle.** Confirm switching between backup
      and restore direction actually swaps which side is source vs.
      destination in the real run, not just in the label.
- [ ] **History screen.** After a couple of real runs exist, confirm the
      runs list renders, the empty-state gating fixes (`b9fb5dc`,
      `2e22763` — empty state should only show when not loading and not
      errored, never flash incorrectly) look right, and understand that
      "Not synced" folder rows will likely all show up as not-synced today
      because of the `folder_rules` persistence gap (Section 0 item 2) —
      don't file that as a new bug, it's the known gap.
- [ ] **Profile settings screen.** Edit a saved device profile, confirm the
      Save button is disabled with no changes pending (`450a4d1`), enabled
      once something changes, and that per-device saving state tracked as a
      set (`32a4266`) doesn't leak "saving" spinners across unrelated device
      rows when editing two profiles in quick succession.

## 5. Already verified during implementation — do not re-verify from scratch

Listed so QA time isn't wasted re-proving things already checked against
real inputs during Tasks 1-15. Spot-confirm at most, don't deep-dive:

- `adb shell ls -la` (toybox) output parsing — verified against a real
  connected device in `41c2f6a` (classification/`device_scan.rs`), matches
  `docs/research/android-storage-domain.md`'s worked example exactly,
  including the `WhatsApp` shadow-duplicate case.
- `adb shell du -s` output parsing (the *happy-path* format) — verified
  against a real connected device in `space.rs` doc comments, e.g.
  `"4\t/storage/emulated/0/Alarms"`. (The *permission-denied* path is the
  unverified part — see Section 3.)
- adbsync's real stdout/stderr line formats (`[ 45%] <path>` progress,
  `[CRITICAL] <message>` on Windows) — reconciled against the vendored
  better-adb-sync v1.4.0 source and strings extracted from the frozen
  `adb.exe` binary in `ac6c9f8`, not guessed.
- The "never report success when an Error/Fatal event was observed" rule —
  has full unit test coverage of the decision matrix (`3cef699`), including
  the zero-exit-but-observed-error regression case.
- The free-space-vs-estimate boundary condition (`free_bytes ==
  estimated_bytes`) — explicitly locked by a test in `5097ca0`.
- Migration SQL itself (schema correctness, insert/select round-trip) —
  proven against a real sqlite file via a standalone harness in `862d3e3`.
  (Only the *registration into the real Tauri app* was unverified — see
  Section 0 item 3 / Section 1.)

## 6. Sign-off

Record for each Priority 0/1 item above: date, device model + Android
version, destination volume/filesystem, pass/fail, and (for any fail) a link
to the filed issue. This feature should not be considered ready for real
users until Section 1 and Section 3 are both fully green on real hardware —
those are the two places where "it passed in a test harness" and "it works
for a person with a full phone and a nearly-full drive" have not yet been
shown to be the same thing.
