# Android Shared Storage — Domain Knowledge

## Layout

- `/sdcard` is a symlink to `/storage/emulated/0`.
- `/storage/emulated/` can hold multiple numbered profiles, not just `0`:
  - `0` — Owner/primary profile. The one that matters for a personal backup.
  - Other IDs (e.g. `10`, `999`) — secondary profiles such as Private Space
    or Dual Apps. **Never confirmed whether these are readable over ADB** on
    the test device — Private Space in particular is commonly locked. See
    [app-requirements.md](./app-requirements.md) open items.
  - `obb` — shared OBB (game/app expansion data) area, separate from any
    profile.
- No physical SD card is assumed; only the internal partition and a FUSE
  mount at `/storage/emulated`. A real implementation should not assume this
  and should detect storage volumes rather than hardcoding paths.

## Recurring folder classification pattern

Within a profile, top-level folders reliably fall into a few buckets. This
pattern held on the one device investigated so far and is a reasonable
starting heuristic, but **is not guaranteed to generalize** — see
[app-requirements.md](./app-requirements.md) for why this needs to stay an
editable, per-device artifact rather than a hardcoded list.

| Bucket | Examples | Default | Signal |
|---|---|---|---|
| Personal media | `DCIM`, `Pictures`, `Movies`, `Music`, `Documents`, `Download` | Include | Standard Android media dirs, always user content |
| Stock/empty noise | `Alarms`, `Notifications`, `Audiobooks`, `Podcasts`, `Recordings` | Skip | Usually empty or vendor-shipped sound packs |
| App-private regenerable caches | Camera-mod configs (`Gcam`, `SGCAM`), app debug logs, downloaded-but-re-fetchable content (e.g. manga chapters) | Skip | No structural signal — requires human judgment or a heuristic (mtime never changes + large + matches a known app package) |
| Stale duplicate of `Android/media/<pkg>` data | Root `/WhatsApp` vs. `Android/media/com.whatsapp/WhatsApp` | Prefer newer mtime | Any top-level folder name that also matches a package's media folder name is suspect — compare mtimes, the older one is usually a stale leftover from before the app switched to scoped storage |
| Restricted app data | `Android/data/<pkg>/...`, `Android/obb/<pkg>/...` | Never bulk-walk | Contains root-restricted subdirectories; a recursive `ls` over these hits `Permission denied` on some paths. Must be excluded from any directory walk, not just filtered after listing — see [adbsync-tooling.md](./adbsync-tooling.md) |

### Concrete example (test device)

Investigated device `00070344C000047`, `/storage/emulated/0` (Owner
profile), all folders owned by `u0_a220`. This table is the *artifact* of
applying the classification above to one real phone — kept here as a worked
example, not as a rule to hardcode:

| Folder | Contents | Decision |
|---|---|---|
| `Alarms` | empty | Skip |
| `Notifications` | 19× stock OEM sound pack | Skip |
| `Ringtones/Compositions` | 3 files, one is a real custom ringtone | Partial — only `Invisible .ogg` |
| `Audiobooks`, `Podcasts`, `Recordings` | empty | Skip |
| `DCIM`, `Pictures`, `Movies`, `Music`, `Documents`, `Download` | personal media | Include |
| `Gcam`, `SGCAM` | camera mod/app configs | Skip |
| `Dartotsu/appLogs.txt` | debug log | Skip |
| `Mihon` (`autobackup`, `downloads` 104M) | manga app library + downloaded chapters | Skip (whole folder) |
| `SUYU/prod.keys`, `title.keys` | emulator crypto keys, not regenerable | Include |
| `SwiftBackup` | app backups, actively being added to | Include (whole folder) |
| `Stuff/*.jpg` | personal photo | Include |
| root `/WhatsApp` | stale, last modified 2024-04-18 | Skip (superseded) |
| `Android/media/com.whatsapp/WhatsApp` | active, modified same day as investigation | Include — this is the real WhatsApp data (`Databases`, `Backups`, `Media`) |
| `Android/media/{com.Slack,com.instagram.android,com.openai.chatgpt,org.telegram.messenger}` | empty or stock notification sounds | Skip |

Full include/exclude list for this device: see `phone-backup-excludes.txt`
in the original scratch folder (`~/Desktop/files/platform-tools/`) for the
literal pattern list — noted in
[adbsync-tooling.md](./adbsync-tooling.md) as not directly usable with
`--exclude-from` due to the whole-tree-walk bug.
