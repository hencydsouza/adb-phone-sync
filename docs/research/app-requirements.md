# Requirements & Design Constraints for Backup/Restore/Sync

Concrete requirements/design hints surfaced directly by the research in this
directory, for whenever this feature gets scoped and designed. Nothing about
the actual UI/architecture is decided yet — this is input for that
discussion, not a spec. Per the original handoff note, start design with the
`superpowers:brainstorming` skill rather than jumping to implementation.

## Requirements derived from confirmed bugs/incidents

- **Never expose raw ANDROID/LOCAL path pairs to the user.** Construct them
  internally (with correct trailing-slash handling) from a per-folder
  include list, so the nesting footgun
  ([adbsync-tooling.md](./adbsync-tooling.md) Bug 1) can't recur.
- **Check destination free space before starting, and mid-transfer** (per
  file or per folder), and stop cleanly with a clear error rather than
  letting individual file writes fail silently one by one. See
  [incident-disk-full-nested-duplicates.md](./incident-disk-full-nested-duplicates.md).
- **Sync primitive = one call per included leaf path**, not a single
  whole-tree call with excludes. The permission-denied whole-tree-walk crash
  ([adbsync-tooling.md](./adbsync-tooling.md) Bug 2) rules out the naive
  approach entirely.
- **Warn on cloud-synced destinations** (OneDrive/Dropbox/etc.), or default
  the destination picker elsewhere. See
  [windows-gotchas.md](./windows-gotchas.md).
- **Any local cleanup/dedup action must verify against the device first**
  before deleting anything — the confirm-before-delete pattern in
  [incident-disk-full-nested-duplicates.md](./incident-disk-full-nested-duplicates.md)
  (structural anomaly → size check → verify against device → only then
  delete), never delete based on local heuristics alone.
- **`--adb-bin` (or equivalent explicit ADB binary path) should always be
  passed/used explicitly**, never rely on ambient `PATH` — this already
  caused a failure mode (`FileNotFoundError: [WinError 2]`) during manual
  use.

## Requirements derived from domain knowledge

- **Per-device include/exclude classification is a first-class, editable
  artifact**, not a hard-coded list. The one worked example in
  [android-storage-domain.md](./android-storage-domain.md) took a full
  manual investigation pass and won't transfer unchanged to a different
  phone's folder set. The app needs a UI/flow for reviewing and editing this
  classification per device, not just running a fixed script.
- **Stale-duplicate detection**: be suspicious of any top-level folder name
  that also appears as a package's `Android/media/<pkg>` folder name (e.g.
  root `WhatsApp` vs. `Android/media/com.whatsapp/WhatsApp`), and prefer
  whichever has the newer mtime.
- **`Android/data` and `Android/obb` must never be walked recursively** as
  part of building a file listing — they contain root-restricted
  per-app subdirectories that will always break a naive recursive `ls`.
- **No `--del`-equivalent behavior by default.** Deleting PC-side files
  missing from the phone turns a backup into a mirror — should be opt-in and
  require explicit confirmation, never the default.

## Open decision: shell out vs. reimplement

Whether the underlying sync engine should stay as `adbsync` (shelled out to
from Rust, passing `--adb-bin` explicitly) or be reimplemented natively in
Rust for this app. Reimplementing would fix the trailing-slash and
whole-tree-exclude bugs at the source instead of working around them, at the
cost of reimplementing ADB's sync protocol handling. Not decided.

## Open items (not yet resolved by research)

- Whether secondary Android profiles (Private Space, Dual Apps — see
  [android-storage-domain.md](./android-storage-domain.md)) are even
  readable over ADB. Flagged early in the original investigation, never
  actually tested. The app should probe for these and clearly report
  "inaccessible" rather than silently skipping, once tested.
- Whether mtime-only change detection (inherent to `adbsync`, or whatever
  replaces it — see [adbsync-tooling.md](./adbsync-tooling.md)) is
  sufficient, or whether the app should add its own integrity check (hash
  comparison) on top. This matters especially after an incident like
  [incident-disk-full-nested-duplicates.md](./incident-disk-full-nested-duplicates.md),
  where files could have been partially written when a disk filled up
  mid-write, yet still look "in sync" by mtime alone.
- Whether a native Rust ADB invocation path reintroduces any equivalent of
  the Git Bash/MSYS path-mangling issue — see
  [windows-gotchas.md](./windows-gotchas.md). Needs a smoke test once that
  code exists.
