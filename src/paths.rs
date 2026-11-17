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

/// Security checks on an existing socket: directory owned by us and not group/world writable; the socket
/// itself not accessible by others. Returns a list of problems (empty when fine).
pub fn permission_problems(path: &Path) -> Vec<String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mut out = Vec::new();
    let uid = unsafe { libc_getuid() };
    if let Some(dir) = path.parent()
        && let Ok(m) = std::fs::metadata(dir)
    {
        if m.uid() != uid {
            out.push(format!(
                "socket directory {} is not owned by you",
                dir.display()
            ));
        }
        if m.permissions().mode() & 0o022 != 0 {
            out.push(format!(
                "socket directory {} is group/world writable (mode {:o})",
                dir.display(),
                m.permissions().mode() & 0o777
            ));
        }
    }
    if let Ok(m) = std::fs::symlink_metadata(path) {
        if m.uid() != uid {
            out.push("socket file is not owned by you".into());
        }
        if m.permissions().mode() & 0o077 != 0 {
            out.push(format!(
                "socket file is accessible by others (mode {:o}); expected 600",
                m.permissions().mode() & 0o777
            ));
        }
    }
    out
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
