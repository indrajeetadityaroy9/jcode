use std::path::Path;

/// Set file permissions to owner read/write/execute (0o755).
pub fn set_permissions_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(path, perms)
}

/// Atomically swap a symlink by creating a temp symlink and renaming.
///
/// Creates a temp symlink, then renames over the target (atomic).
pub fn atomic_symlink_swap(src: &Path, dst: &Path, temp: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(temp);
    std::os::unix::fs::symlink(src, temp)?;
    std::fs::rename(temp, dst)?;
    Ok(())
}
