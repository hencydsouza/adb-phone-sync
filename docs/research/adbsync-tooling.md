# `better-adb-sync` (`adbsync`) — Confirmed Behavior

Findings below were confirmed by using the tool directly and by reading its
source (`BetterADBSync/FileSystems/Base.py`, `BetterADBSync/__init__.py`),
not just its README/docs.

## Identity and install

- PyPI package name is **`BetterADBSync`**; the installed command is
  **`adbsync`** — not `adb-sync`, which is a different, unrelated tool with a
  similar name. Easy to conflate when searching for it.
- Installed via `pip install BetterADBSync`. Does not add itself to PATH.

## CLI shape

```
adbsync pull ANDROID LOCAL
adbsync push LOCAL ANDROID
```

- Global flags (`-n` dry-run, `--show-progress`, `--adb-bin`, `--exclude`,
  `--exclude-from`) must come **before** the `pull`/`push` subcommand.
  Putting them after produces `unrecognized arguments`.
- Shells out to whatever `adb` binary is on `PATH` by default. If the
  calling process's `PATH` doesn't include `adb`, it fails with
  `FileNotFoundError: [WinError 2]`. **A packaged app should always pass
  `--adb-bin` explicitly** rather than relying on ambient `PATH`.

## Change detection

**mtime-only, rounded to the minute.** No size or hash comparison, no
block-level delta — a changed file is fully re-transferred, never
partially (unlike real rsync). Practical implications:

- Re-running the same pull commands later is safe and genuinely
  incremental — unchanged files are skipped.
- A corrupted-but-newer-mtime file will look "in sync" even though its
  content is wrong. No integrity verification happens by default.
- Deletions on the local side only happen if `--del` is passed. Not using
  it keeps a workflow additive/backup-safe rather than mirror-like.

## Bug 1 — trailing-slash nesting footgun (confirmed, hit in production)

`adbsync pull ANDROID LOCAL` follows rsync-style trailing-slash semantics:
if `ANDROID` has **no** trailing slash, the tool copies the source folder
**itself** into `LOCAL`, rather than syncing its contents into `LOCAL`. This
matches the tool's own README example (`adb-sync ~/Music /sdcard` creates
`/sdcard/Music`), but is a footgun when `LOCAL` already names the target
folder:

```
# WRONG — creates $dest\DCIM\DCIM\... (nested duplicate)
adbsync pull "/storage/emulated/0/DCIM" "$dest\DCIM"

# CORRECT — contents land directly in $dest\DCIM
adbsync pull "/storage/emulated/0/DCIM/" "$dest\DCIM"
```

There is **no warning or error** when this happens — it silently creates the
nested duplicate. Only caught by manually inspecting the destination tree.
See [incident-disk-full-nested-duplicates.md](./incident-disk-full-nested-duplicates.md)
for the real incident this caused (18GB of duplicated data in one case).

**Implication for a wrapper tool:** never expose raw `ANDROID`/`LOCAL` path
pairs to the user as free text. Construct both internally (with correct
trailing-slash handling) from a per-folder include list.

## Bug 2 — whole-tree walk crashes on restricted subfolders (confirmed)

`adbsync pull /storage/emulated/0 <dest> --exclude-from excludes.txt`
crashes:

```
[CRITICAL] ADB line not captured
ls: .../Android/data/org.videolan.vlc/files/medialib: Permission denied
[CRITICAL] Exiting
```

**Root cause (confirmed by reading source):** `adbsync` performs one
recursive `ls` walk over the *entire* source path first to build its file
listing, and only applies `--exclude`/`--exclude-from` patterns to that
listing **afterward** — it cannot skip a directory during the walk itself.
Since `Android/data/<pkg>/...` and `Android/obb/<pkg>/...` contain
root-restricted subdirectories, the walk hits `Permission denied` output
mixed into stdout, and the whole tool dies.

**Consequence:** whole-tree pulls with excludes do not work if the tree
contains any restricted subfolder — which `/storage/emulated/0` always does
via `Android/data`/`Android/obb`. `phone-backup-excludes.txt` (the exclude
pattern list drafted for the test device) is useful as a *reference* for
what was decided to skip, but **cannot actually be used** as an
`--exclude-from` argument against a source that includes those paths.

**Working approach, confirmed reliable:** pull each *included* top-level
path individually (one `adbsync pull` call per path), so the walk never
enters a restricted directory at all.

**Implication for a wrapper tool:** the core sync primitive should be "one
call per included leaf path," not "one call with an exclude list."

## Restore direction

`adbsync push LOCAL ANDROID` is the reverse operation, only relevant when
restoring to a new/wiped device. Not yet tested in practice. Note the
exclude-pattern direction doesn't necessarily line up 1:1 with the pull
direction — patterns collected for pull (relative to the Android-side
source) would need review before reuse for a push, since by restore time the
PC-side structure may not mirror the original phone-side structure exactly.
