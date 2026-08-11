# Research: Android Backup/Restore/Sync

This directory consolidates research gathered from a manual `adbsync`
(better-adb-sync) backup effort, done ahead of building this app's
backup/restore/sync feature. Source material was scratch notes in
`~/Desktop/files/platform-tools/` (`HANDOFF.md`, `backup-plan.md`,
`phone-backup-learnings.md`, `phone-backup-excludes.txt`, and a Claude Code
session transcript) — this directory is the cleaned-up, permanent version of
that research, scoped for this codebase.

No backup/restore/sync code exists in this app yet. This is pre-implementation
research: domain knowledge, a specific tool's confirmed behavior (including
two bugs hit in production use), and the requirements those findings imply.

## Contents

- **[android-storage-domain.md](./android-storage-domain.md)** — Android
  shared storage layout, multi-profile structure, and the recurring folder
  classification pattern (personal media / stock noise / app caches / stale
  duplicates / restricted app data).
- **[adbsync-tooling.md](./adbsync-tooling.md)** — confirmed behavior of
  `better-adb-sync` (the `adbsync` CLI): syntax, change-detection semantics,
  and two confirmed bugs (trailing-slash nesting footgun, whole-tree exclude
  crash) with root causes traced to source.
- **[windows-gotchas.md](./windows-gotchas.md)** — Windows/Git-Bash/OneDrive
  environment issues encountered, and which are expected to disappear in a
  native Rust/Tauri implementation vs. which need explicit handling.
- **[incident-disk-full-nested-duplicates.md](./incident-disk-full-nested-duplicates.md)**
  — case study of a real incident (disk-full + nested-duplicate-folder bug
  compounding into a confusing symptom) and the diagnostic method that
  resolved it. Useful as a template for how the app should validate/diagnose
  problems rather than just executing commands blindly.
- **[app-requirements.md](./app-requirements.md)** — the concrete
  requirements/design constraints these findings imply for this app's
  backup/restore/sync feature, plus open items that still need resolving.

## How to use this

Start with `app-requirements.md` for the actionable summary. Use the other
docs as supporting detail/citations when a requirement's rationale needs
double-checking. Per the original handoff note, design work for this feature
should start with the `superpowers:brainstorming` skill (not straight to
implementation) using `app-requirements.md` as input — nothing about the
actual UI/architecture has been designed yet, only the constraints are known.
