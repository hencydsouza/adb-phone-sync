/// A classification decision for a top-level (or shadow-checked) folder on
/// the device, produced by [`classify`] / [`classify_with_mtime_hint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Decision {
    /// Personal content — back it up.
    Include,
    /// Vendor noise / empty / app-regenerable — leave it out.
    Skip,
    /// A root-level folder that's shadowed by a newer copy under
    /// `Android/media/<pkg>/...` — the root copy is a stale leftover from
    /// before the owning app switched to scoped storage.
    SkipStaleDuplicate,
}

/// A single folder entry as reported by a device listing (e.g. `adb shell
/// ls`). This module only classifies; it does not walk the device itself.
///
/// `name`'s expected shape depends on how the entry is used:
/// - As the `entry` being classified, `name` is a bare top-level folder name
///   (e.g. `"DCIM"`, `"WhatsApp"`).
/// - As a member of the `siblings` list passed to [`classify`] /
///   [`classify_with_mtime_hint`], `name` must be the full relative path
///   (e.g. `"Android/media/com.whatsapp/WhatsApp"`) so the shadow-duplicate
///   check can match it against `Android/media/<pkg>/<entry-name>`. A bare
///   name in `siblings` will simply never match and is otherwise harmless.
#[derive(Debug)]
pub struct FolderEntry {
    pub name: String,
    /// Reserved for future use: not yet consulted by [`classify`] /
    /// [`classify_with_mtime_hint`]. Required today only because real
    /// device-listing wiring (not yet implemented) will need it once this
    /// module walks actual `adb shell ls -R` output.
    pub is_dir: bool,
    pub is_empty: bool,
}

/// Standard Android media directories — always user content, always include.
const PERSONAL_MEDIA: &[&str] = &["DCIM", "Pictures", "Movies", "Music", "Documents", "Download"];

/// Stock/vendor-shipped folders that are usually empty or noise.
const STOCK_NOISE: &[&str] = &["Alarms", "Notifications", "Audiobooks", "Podcasts", "Recordings"];

/// Restricted app-data trees. Contain root-restricted subdirectories that a
/// recursive walk can't fully read; must be excluded outright, not just
/// filtered after listing.
const ALWAYS_EXCLUDED_PREFIXES: &[&str] = &["Android/data", "Android/obb"];

/// Classify a folder, assuming it is *not* older than any shadow copy under
/// `Android/media/<pkg>/...`. Convenience wrapper over
/// [`classify_with_mtime_hint`] for callers that haven't compared mtimes.
pub fn classify(entry: &FolderEntry, siblings: &[FolderEntry]) -> Decision {
    classify_with_mtime_hint(entry, siblings, true)
}

/// Classify a folder using the recurring pattern documented in
/// `docs/research/android-storage-domain.md`.
///
/// `siblings` is the rest of the folder listing (used only for the
/// stale-duplicate check against `Android/media/<pkg>/<name>`).
/// `is_newer_than_shadow` should be `true` when `entry`'s mtime is newer
/// than (or there is no) matching `Android/media` shadow copy; `false` when
/// a newer shadow copy exists, which marks `entry` as a stale duplicate.
pub fn classify_with_mtime_hint(
    entry: &FolderEntry,
    siblings: &[FolderEntry],
    is_newer_than_shadow: bool,
) -> Decision {
    if ALWAYS_EXCLUDED_PREFIXES
        .iter()
        .any(|p| entry.name == *p || entry.name.starts_with(&format!("{p}/")))
    {
        return Decision::Skip;
    }
    if PERSONAL_MEDIA.contains(&entry.name.as_str()) {
        return Decision::Include;
    }
    if STOCK_NOISE.contains(&entry.name.as_str()) || entry.is_empty {
        return Decision::Skip;
    }
    // Stale-duplicate check: a top-level folder name that also appears as
    // `Android/media/<pkg>/<name>` is suspect — prefer whichever is newer.
    // Note: this suffix match assumes the documented 2-level
    // `Android/media/<pkg>/<name>` shape; it would also (incorrectly) match a
    // deeper `Android/media/<pkg>/<subdir>/<name>` sibling. Low practical
    // risk in observed device listings, but worth knowing if false positives
    // show up once real device data is wired in.
    let shadow_suffix = format!("/{}", entry.name);
    let has_media_shadow = siblings
        .iter()
        .any(|s| s.name.starts_with("Android/media/") && s.name.ends_with(&shadow_suffix));
    if has_media_shadow && !is_newer_than_shadow {
        return Decision::SkipStaleDuplicate;
    }
    // Default: unknown folder with no structural signal either way — include
    // and let the human review it in the classification screen.
    Decision::Include
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, is_empty: bool) -> FolderEntry {
        FolderEntry { name: name.to_string(), is_dir, is_empty }
    }

    #[test]
    fn personal_media_folders_are_included() {
        for name in ["DCIM", "Pictures", "Movies", "Music", "Documents", "Download"] {
            let suggestion = classify(&entry(name, true, false), &[]);
            assert_eq!(suggestion, Decision::Include, "{name} should be included");
        }
    }

    #[test]
    fn stock_noise_folders_are_skipped() {
        for name in ["Alarms", "Notifications", "Audiobooks", "Podcasts", "Recordings"] {
            let suggestion = classify(&entry(name, true, true), &[]);
            assert_eq!(suggestion, Decision::Skip, "{name} should be skipped");
        }
    }

    #[test]
    fn android_data_and_obb_are_always_excluded() {
        assert_eq!(classify(&entry("Android/data", true, false), &[]), Decision::Skip);
        assert_eq!(classify(&entry("Android/obb", true, false), &[]), Decision::Skip);
    }

    #[test]
    fn root_folder_shadowed_by_newer_android_media_copy_is_flagged_stale() {
        // root /WhatsApp (older) vs Android/media/com.whatsapp/WhatsApp (newer)
        let siblings = [entry("Android/media/com.whatsapp/WhatsApp", true, false)];
        let suggestion = classify_with_mtime_hint(&entry("WhatsApp", true, false), &siblings, false /* not newer */);
        assert_eq!(suggestion, Decision::SkipStaleDuplicate);
    }

    #[test]
    fn unknown_folder_with_no_signal_defaults_to_include() {
        // No structural signal either way: not personal media, not stock
        // noise, not empty, no Android/media shadow. Locks in the reviewed
        // scope decision to default to Include and let a human decide.
        let suggestion = classify(&entry("SomeRandomAppFolder", true, false), &[]);
        assert_eq!(suggestion, Decision::Include);
    }

    #[test]
    fn newer_than_shadow_is_included_despite_matching_shadow() {
        // root /WhatsApp is newer than (or equal to) the Android/media
        // shadow copy, so it should NOT be flagged as a stale duplicate.
        let siblings = [entry("Android/media/com.whatsapp/WhatsApp", true, false)];
        let suggestion = classify_with_mtime_hint(&entry("WhatsApp", true, false), &siblings, true /* newer */);
        assert_eq!(suggestion, Decision::Include);
    }
}
