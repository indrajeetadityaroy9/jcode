use super::COMMAND_EXISTS_CACHE;

pub(crate) fn command_exists(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    // Absolute/relative path: direct stat, no caching needed
    let path = std::path::Path::new(command);
    if path.is_absolute() || contains_path_separator(command) {
        return explicit_command_exists(path);
    }

    // Check per-process cache first (O(1) on repeated calls)
    if let Ok(cache) = COMMAND_EXISTS_CACHE.lock()
        && let Some(&cached) = cache.get(command)
    {
        return cached;
    }

    let path_var = match std::env::var_os("PATH") {
        Some(p) if !p.is_empty() => p,
        _ => {
            cache_command_result(command, false);
            return false;
        }
    };

    let found = std::env::split_paths(&path_var)
        .flat_map(|dir| {
            command_candidates(command)
                .into_iter()
                .map(move |c| dir.join(c))
        })
        .any(|p| p.exists());

    cache_command_result(command, found);
    found
}

fn cache_command_result(command: &str, exists: bool) {
    if let Ok(mut cache) = COMMAND_EXISTS_CACHE.lock() {
        cache.insert(command.to_string(), exists);
    }
}

fn explicit_command_exists(path: &std::path::Path) -> bool {
    path.exists()
}

pub(crate) fn command_candidates(command: &str) -> Vec<std::ffi::OsString> {
    let path = std::path::Path::new(command);
    let file_name = match path.file_name() {
        Some(name) => name.to_os_string(),
        None => return Vec::new(),
    };

    if has_extension(path) {
        return vec![file_name];
    }

    let candidates = vec![file_name];

    dedup_preserve_order(candidates)
}

pub(crate) fn contains_path_separator(command: &str) -> bool {
    command.contains('/')
        || command.contains('\\')
        || std::path::Path::new(command).components().count() > 1
}

pub(crate) fn has_extension(path: &std::path::Path) -> bool {
    path.extension().is_some()
}

pub(crate) fn dedup_preserve_order(mut values: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut out = Vec::new();
    for value in values.drain(..) {
        if !out.iter().any(|v| v == &value) {
            out.push(value);
        }
    }

    out
}
