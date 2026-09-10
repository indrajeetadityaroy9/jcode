use std::sync::LazyLock;

/// A changelog entry: commit hash plus subject.
#[derive(Clone, Copy)]
pub(super) struct ChangelogEntry<'a> {
    pub hash: &'a str,
    pub subject: &'a str,
}

/// Parse changelog entries from the embedded changelog string.
///
/// Format per entry: "hash<RS>subject", entries separated by the ASCII unit
/// separator (0x1F). `git log --format=%h%x1e%s` produces the record directly;
/// see `crates/jcode-build-meta/build.rs`.
///
/// A record with any other field count is rejected rather than guessed at. Three
/// producers write this format (that build script plus `scripts/remote_build.sh`
/// and `scripts/build_linux_compat.sh`), so one drifting out of step must yield
/// an empty Updates box, never subjects showing the wrong field.
#[cfg(test)]
pub(super) fn parse_changelog_from(changelog: &str) -> Vec<ChangelogEntry<'_>> {
    parse_changelog_from_impl(changelog)
}

fn parse_changelog_from_impl(changelog: &str) -> Vec<ChangelogEntry<'_>> {
    if changelog.is_empty() {
        return Vec::new();
    }
    changelog
        .split('\x1f')
        .filter_map(|entry| {
            let mut fields = entry.split('\x1e');
            let hash = fields.next()?;
            let subject = fields.next()?;
            if fields.next().is_some() {
                return None;
            }
            Some(ChangelogEntry { hash, subject })
        })
        .collect()
}

/// Parse the embedded changelog from the build-time environment.
fn parse_changelog() -> Vec<ChangelogEntry<'static>> {
    let changelog: &'static str = jcode_build_meta::CHANGELOG;
    parse_changelog_from_impl(changelog)
}

/// Changelog subjects the user has not seen yet, computed once per process.
///
/// Reads the last-seen commit hash from ~/.jcode/last_seen_changelog, keeps
/// only entries newer than it, then records the latest hash so the next
/// process starts from here.
static UNSEEN_ENTRIES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let all_entries = parse_changelog();
    if all_entries.is_empty() {
        return Vec::new();
    }

    let state_file = dirs::home_dir()
        .map(|h| h.join(".jcode").join("last_seen_changelog"))
        .unwrap_or_else(|| std::path::PathBuf::from(".jcode/last_seen_changelog"));

    let last_seen_hash = std::fs::read_to_string(&state_file)
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let new_entries: Vec<String> = if last_seen_hash.is_empty() {
        all_entries
            .iter()
            .take(5)
            .map(|e| e.subject.to_string())
            .collect()
    } else {
        all_entries
            .iter()
            .take_while(|e| e.hash != last_seen_hash)
            .map(|e| e.subject.to_string())
            .collect()
    };

    if let Some(first) = all_entries.first() {
        if let Some(parent) = state_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&state_file, first.hash);
    }

    new_entries
});

/// Get changelog entries the user hasn't seen yet.
/// Reads the last-seen commit hash from ~/.jcode/last_seen_changelog,
/// filters the embedded changelog to only new entries, then saves the latest hash.
/// Returns just the commit subjects (not the hashes).
pub(super) fn get_unseen_changelog_entries() -> &'static Vec<String> {
    &UNSEEN_ENTRIES
}
