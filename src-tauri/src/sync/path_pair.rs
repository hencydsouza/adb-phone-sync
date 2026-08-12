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
    fn empty_source_yields_just_the_separator() {
        // Documents the known limitation noted on `with_trailing_slash`:
        // empty input isn't rejected, it just becomes the bare separator.
        // Validation of non-empty paths is expected to happen upstream.
        let pair = build_pull_pair("", r"C:\dest\DCIM");
        assert_eq!(pair.android(), "/");
    }
}
