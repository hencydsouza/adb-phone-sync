/// Result of comparing an estimated transfer size against the destination's
/// available free space.
pub struct SpaceCheck {
    /// `true` when `free_bytes` is strictly greater than `estimated_bytes` —
    /// i.e. the transfer would leave at least some headroom on the
    /// destination volume. Exactly-equal counts as NOT enough space: a
    /// transfer that consumes every last free byte leaves zero margin for
    /// filesystem metadata overhead, other concurrent writes, or the
    /// estimate being slightly off, so it's treated as a preflight failure
    /// rather than a razor-thin pass.
    pub has_enough_space: bool,
    pub free_bytes: u64,
    pub estimated_bytes: u64,
}

/// Compare an estimated transfer size (`estimated_bytes`, e.g. from `adb
/// shell du` on the source) against the destination's available free space
/// (`free_bytes`, e.g. from a disk-space lookup on the local target). Pure
/// decision logic only — callers are responsible for obtaining both values.
pub fn check_space(estimated_bytes: u64, free_bytes: u64) -> SpaceCheck {
    SpaceCheck { has_enough_space: free_bytes > estimated_bytes, free_bytes, estimated_bytes }
}

/// Marker substrings that indicate a path lives inside a cloud-sync client's
/// managed folder. Writing large backup trees into one of these can trigger
/// slow/blocking hydration or sync churn (see
/// `docs/research/windows-gotchas.md`), so callers should warn the user
/// rather than silently proceeding.
const CLOUD_SYNC_MARKERS: &[&str] = &["OneDrive", "Dropbox", "Google Drive", "iCloudDrive"];

/// `true` if `path` appears to be inside a cloud-sync client's managed
/// folder, based on a substring match against [`CLOUD_SYNC_MARKERS`].
pub fn is_cloud_synced_path(path: &str) -> bool {
    CLOUD_SYNC_MARKERS.iter().any(|marker| path.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_when_free_space_is_below_estimate() {
        let result = check_space(10_000, 5_000);
        assert!(!result.has_enough_space);
    }

    #[test]
    fn passes_when_free_space_exceeds_estimate() {
        let result = check_space(5_000, 10_000);
        assert!(result.has_enough_space);
    }

    #[test]
    fn detects_onedrive_paths() {
        assert!(is_cloud_synced_path(r"C:\Users\hency\OneDrive\Desktop\nothing2a"));
        assert!(!is_cloud_synced_path(r"D:\Backups\phone"));
    }

    #[test]
    fn detects_dropbox_paths() {
        assert!(is_cloud_synced_path(r"C:\Users\hency\Dropbox\phone"));
    }
}
