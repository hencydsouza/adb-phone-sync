# Incident: Disk-Full + Nested-Duplicate-Folder Bug (First Real Backup Run)

Case study from the first real `adbsync` backup attempt. Two independent
problems compounded into a single confusing symptom. Kept as a worked
example of the diagnostic method a future app should apply automatically
before taking any destructive local action.

## Symptom

User re-ran the same backup command and it appeared to "go through every
file again" instead of resuming incrementally, with no indication of why.

## Problem 1 — destination drive filled completely

`C:` reached 488G/488G used, 0 bytes free. Every write after that point
failed with `adb: error: cannot write '...': Input/output error`. Because
`adbsync`'s change detection is mtime-based (see
[adbsync-tooling.md](./adbsync-tooling.md)), a file that never finished
writing looks identical to a file that was never copied — so every
re-comparison retried it, forever. From the user's side this was
indistinguishable from "it's not tracking progress at all."

## Problem 2 — nested duplicate folders (separate root cause)

Destination tree contained doubled folder names: `nothing2a\DCIM\DCIM\`,
`nothing2a\WhatsApp\WhatsApp\`, `nothing2a\Stuff\Stuff\`,
`nothing2a\SUYU\SUYU\`, `nothing2a\SwiftBackup\SwiftBackup\`, and smaller
duplicates for `Movies`, `Music`, `Pictures`, `Documents`, `Download`.

Caused by the trailing-slash footgun documented in
[adbsync-tooling.md](./adbsync-tooling.md): the original pull commands used
`ANDROID` source paths with no trailing slash (e.g. `.../DCIM`), so
`adbsync` copied the folder *itself* into the destination.

## Diagnostic method (worked, in order)

1. **Structural anomaly detection:** `find -maxdepth 2` on the destination
   to see the actual tree shape and spot the doubled folder names.
2. **Size check:** `du -sh` on each suspicious nested folder, to see if it
   held real (wasted) data or was negligible. Only `DCIM\DCIM` turned out to
   be significant — 18GB. The rest were a few MB or less.
3. **Verify against source of truth:** `adb shell ls` against the
   corresponding on-device path, confirmed via `MSYS_NO_PATHCONV=1` (see
   [windows-gotchas.md](./windows-gotchas.md)):
   ```
   $ MSYS_NO_PATHCONV=1 ./adb.exe shell ls -la /storage/emulated/0/DCIM/DCIM
   ls: /storage/emulated/0/DCIM/DCIM: No such file or directory
   ```
   Confirmed the nested folder does not exist on the phone at all — it is a
   pure sync-tool artifact, safe to delete. Cross-checked for WhatsApp too:
   the real on-device `Android/media/com.whatsapp/WhatsApp` listing (from
   earlier investigation) has no nested `WhatsApp` subfolder.
4. **Only then delete**, and only the confirmed-safe artifacts:
   ```powershell
   Remove-Item -Recurse -Force "$dest\DCIM\DCIM"
   Remove-Item -Recurse -Force "$dest\WhatsApp\WhatsApp"
   Remove-Item -Recurse -Force "$dest\Stuff\Stuff"
   Remove-Item -Recurse -Force "$dest\SUYU\SUYU"
   Remove-Item -Recurse -Force "$dest\SwiftBackup\SwiftBackup"
   Remove-Item -Recurse -Force "$dest\Movies\Movies"
   Remove-Item -Recurse -Force "$dest\Music\Music"
   Remove-Item -Recurse -Force "$dest\Pictures\Pictures"
   Remove-Item -Recurse -Force "$dest\Documents\Documents"
   Remove-Item -Recurse -Force "$dest\Download\Download"
   ```

## Fix applied going forward

Added a trailing slash to every `ANDROID` source path so contents land
directly in the destination instead of nesting:

```
adbsync pull "/storage/emulated/0/DCIM/" "$dest\DCIM"
```

Freeing the ~18GB from `DCIM\DCIM` was **necessary but not sufficient** —
more space still needed to be freed on `C:` (or the destination moved to a
drive with more room) before the backup could actually complete. Not yet
resolved at time of writing.

## Takeaway for this app

The **confirm-before-delete pattern** used here (structural anomaly → size
check → verify against device as source of truth → only then delete) should
be a standard, automatic operation for any local cleanup this app performs
— never delete based on local heuristics alone. See
[app-requirements.md](./app-requirements.md).
