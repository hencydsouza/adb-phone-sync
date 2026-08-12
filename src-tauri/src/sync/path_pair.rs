#[derive(Debug, PartialEq, Eq)]
pub struct PathPair {
    pub android: String,
    pub local: String,
}

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
        assert_eq!(pair.android, "/storage/emulated/0/DCIM/");
        assert_eq!(pair.local, r"C:\dest\DCIM");
    }

    #[test]
    fn push_pair_always_has_trailing_slash_on_local_source() {
        let pair = build_push_pair(r"C:\dest\DCIM", "/storage/emulated/0/DCIM");
        assert_eq!(pair.local, r"C:\dest\DCIM\");
        assert_eq!(pair.android, "/storage/emulated/0/DCIM");
    }

    #[test]
    fn does_not_double_up_an_existing_trailing_slash() {
        let pair = build_pull_pair("/storage/emulated/0/DCIM/", r"C:\dest\DCIM");
        assert_eq!(pair.android, "/storage/emulated/0/DCIM/");
    }

    #[test]
    fn push_does_not_double_up_an_existing_trailing_backslash() {
        let pair = build_push_pair(r"C:\dest\DCIM\", "/storage/emulated/0/DCIM");
        assert_eq!(pair.local, r"C:\dest\DCIM\");
    }
}
