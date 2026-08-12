/// An ANDROID/LOCAL path pair for an `adbsync` invocation.
///
/// Fields are private and can only be populated via [`build_pull_pair`] or
/// [`build_push_pair`], which guarantee the source side carries a trailing
/// separator. This closes off the direct-construction escape hatch
/// (`PathPair { android, local }`) that would otherwise let callers
/// silently reintroduce the nested-duplicate-folder bug this module exists
/// to prevent.
#[derive(Debug, PartialEq, Eq)]
pub struct PathPair {
    android: String,
    local: String,
}

impl PathPair {
    /// The ANDROID-side path (device path, `/`-separated).
    pub fn android(&self) -> &str {
        &self.android
    }

    /// The LOCAL-side path (host path, `\`-separated on Windows).
    pub fn local(&self) -> &str {
        &self.local
    }
}

/// Validates that `path` is safe to use both as an `adb`/`adbsync` argument
/// and, via [`super::orchestration`]'s `leaf_name`, to construct a local
/// filesystem destination path (`dest_root.join(leaf_name(path))`).
///
/// Rejects two things:
/// - Any path that doesn't start with `/`: these are meant to be absolute
///   Android device paths (e.g. `/storage/emulated/0/DCIM`), never relative
///   ones.
/// - Any path containing a literal `..` path segment: `leaf_name` only takes
///   the last `/`-segment of the raw string with no other validation, then
///   joins it onto the profile's local destination root. A path that is (or
///   ends in) a bare `..` segment could otherwise escape the chosen
///   destination folder by one level (e.g. an included path of
///   `/storage/emulated/0/..` would make `leaf_name` return `".."`, and
///   `dest_root.join("..")` walks up a directory instead of writing inside
///   it).
///
/// Called from the `run_backup`/`run_restore`/`space_check` Tauri commands
/// before `included_paths` is used for anything, so a malformed path is
/// rejected with a clear error instead of silently proceeding.
pub fn validate_android_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!(
            "included path {path:?} must be an absolute Android device path (starting with \"/\")"
        ));
    }
    if path.split('/').any(|segment| segment == "..") {
        return Err(format!(
            "included path {path:?} must not contain a \"..\" path segment"
        ));
    }
    Ok(())
}

/// Runs [`validate_android_path`] over every path in `paths`, short-circuiting
/// on the first invalid one.
pub fn validate_included_paths(paths: &[String]) -> Result<(), String> {
    for path in paths {
        validate_android_path(path)?;
    }
    Ok(())
}

/// Appends `sep` to `path` unless it's already present.
///
/// Note: this only recognizes `sep` itself as a terminator. A path that
/// already ends with the *other* separator (e.g. a forward-slash-terminated
/// local path on Windows) will still get `sep` appended. Also note that an
/// empty `path` returns just `sep` — callers are expected to validate
/// non-empty input upstream.
fn with_trailing_slash(path: &str, sep: char) -> String {
    if path.ends_with(sep) {
        path.to_string()
    } else {
        format!("{path}{sep}")
    }
}

/// Build the ANDROID/LOCAL pair for `adbsync pull`.
/// The ANDROID (source) side always gets a trailing slash, so its *contents*
/// land in `local_dest` rather than nesting the source folder inside it.
pub fn build_pull_pair(android_source: &str, local_dest: &str) -> PathPair {
    PathPair {
        android: with_trailing_slash(android_source, '/'),
        local: local_dest.to_string(),
    }
}

/// Build the LOCAL/ANDROID pair for `adbsync push`.
/// The LOCAL (source) side always gets a trailing slash, mirroring pull.
pub fn build_push_pair(local_source: &str, android_dest: &str) -> PathPair {
    PathPair {
        local: with_trailing_slash(local_source, '\\'),
        android: android_dest.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_pair_always_has_trailing_slash_on_android_source() {
        let pair = build_pull_pair("/storage/emulated/0/DCIM", r"C:\dest\DCIM");
        assert_eq!(pair.android(), "/storage/emulated/0/DCIM/");
        assert_eq!(pair.local(), r"C:\dest\DCIM");
    }

    #[test]
    fn push_pair_always_has_trailing_slash_on_local_source() {
        let pair = build_push_pair(r"C:\dest\DCIM", "/storage/emulated/0/DCIM");
        assert_eq!(pair.local(), r"C:\dest\DCIM\");
        assert_eq!(pair.android(), "/storage/emulated/0/DCIM");
    }

    #[test]
    fn does_not_double_up_an_existing_trailing_slash() {
        let pair = build_pull_pair("/storage/emulated/0/DCIM/", r"C:\dest\DCIM");
        assert_eq!(pair.android(), "/storage/emulated/0/DCIM/");
    }

    #[test]
    fn push_does_not_double_up_an_existing_trailing_backslash() {
        let pair = build_push_pair(r"C:\dest\DCIM\", "/storage/emulated/0/DCIM");
        assert_eq!(pair.local(), r"C:\dest\DCIM\");
    }

    #[test]
    fn push_source_with_forward_slash_still_gets_backslash_appended() {
        // Documents the known limitation noted on `with_trailing_slash`:
        // it only recognizes its own separator as "already terminated", so
        // a push-source path ending in `/` gets a redundant trailing `\`.
        let pair = build_push_pair("C:/dest/DCIM/", "/storage/emulated/0/DCIM");
        assert_eq!(pair.local(), "C:/dest/DCIM/\\");
    }

    #[test]
    fn validate_android_path_accepts_a_normal_absolute_path() {
        assert!(validate_android_path("/storage/emulated/0/DCIM").is_ok());
    }

    #[test]
    fn validate_android_path_rejects_a_relative_path() {
        let err = validate_android_path("DCIM").unwrap_err();
        assert!(err.contains("absolute"));
    }

    #[test]
    fn validate_android_path_rejects_a_bare_dotdot_segment() {
        let err = validate_android_path("/storage/emulated/0/..").unwrap_err();
        assert!(err.contains(".."));
    }

    #[test]
    fn validate_android_path_rejects_a_dotdot_segment_in_the_middle() {
        let err = validate_android_path("/storage/../etc").unwrap_err();
        assert!(err.contains(".."));
    }

    #[test]
    fn validate_android_path_accepts_a_path_containing_but_not_equal_to_dotdot() {
        // A folder literally named e.g. "foo..bar" is a legitimate segment,
        // distinct from a `..` traversal segment -- only an exact `..`
        // segment should be rejected.
        assert!(validate_android_path("/storage/emulated/0/foo..bar").is_ok());
    }

    #[test]
    fn validate_included_paths_short_circuits_on_the_first_bad_path() {
        let paths = vec![
            "/storage/emulated/0/DCIM".to_string(),
            "/storage/emulated/0/..".to_string(),
        ];
        assert!(validate_included_paths(&paths).is_err());
    }

    #[test]
    fn validate_included_paths_accepts_an_all_good_list() {
        let paths = vec![
            "/storage/emulated/0/DCIM".to_string(),
            "/storage/emulated/0/Pictures".to_string(),
        ];
        assert!(validate_included_paths(&paths).is_ok());
    }

    #[test]
    fn empty_source_yields_just_the_separator() {
        // Documents the known limitation noted on `with_trailing_slash`:
        // empty input isn't rejected, it just becomes the bare separator.
        // Validation of non-empty paths is expected to happen upstream.
        let pair = build_pull_pair("", r"C:\dest\DCIM");
        assert_eq!(pair.android(), "/");
    }
}
