//! Socket path resolution and directory checks (mirrors `addon/omnix_blender/paths.py`).

use std::path::{Path, PathBuf};

pub const MAX_SOCKET_PATH: usize = 103;

pub fn default_socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("OMNIX_BLENDER_SOCKET").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("omnix-blender")
        .join("blender.sock")
}

pub fn check_len(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let n = path.as_os_str().as_bytes().len();
    if n > MAX_SOCKET_PATH {
        return Err(format!(
            "socket path is {n} bytes; Unix socket paths are limited to {MAX_SOCKET_PATH}. \
             Set OMNIX_BLENDER_SOCKET to a shorter path (same value for the add-on and this server)."
        ));
    }
    Ok(())
}

/// Security checks on an existing socket for `doctor`: everything `untrusted_reason` finds, plus the socket's
/// own mode. Returns a list of problems (empty when fine).
pub fn permission_problems(path: &Path) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;
    let mut out: Vec<String> = untrusted_reason(path).into_iter().collect();
    if let Ok(m) = std::fs::symlink_metadata(path)
        && m.permissions().mode() & 0o077 != 0
    {
        out.push(format!(
            "socket file is accessible by others (mode {:o}); expected 600",
            m.permissions().mode() & 0o777
        ));
    }
    out
}

/// Problems that make an existing socket untrustworthy for a client: its directory is a symlink, not ours, or
/// accessible by group/others (same rule the add-on enforces), or the socket file is not ours.
pub fn untrusted_reason(path: &Path) -> Option<String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let uid = unsafe { libc::getuid() };
    if let Some(dir) = path.parent()
        && let Ok(m) = std::fs::symlink_metadata(dir)
    {
        if m.file_type().is_symlink() {
            return Some(format!("socket directory {} is a symlink", dir.display()));
        }
        if m.uid() != uid {
            return Some(format!(
                "socket directory {} is not owned by you",
                dir.display()
            ));
        }
        if m.permissions().mode() & 0o077 != 0 {
            return Some(format!(
                "socket directory {} is accessible by group/others (mode {:o}); it must be private: chmod 700",
                dir.display(),
                m.permissions().mode() & 0o777
            ));
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.uid() != uid => Some("socket file is not owned by you".into()),
        _ => None,
    }
}
