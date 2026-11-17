//! Locating Blender, packaging the bundled add-on, and installing it through Blender's own CLI.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ADDON_ID: &str = "omnix_blender";

/// Files of the add-on, embedded at build time. `addon_files_match_disk` (unit test) keeps this list honest.
pub const ADDON_FILES: &[(&str, &str)] = &[
    (
        "blender_manifest.toml",
        include_str!("../addon/omnix_blender/blender_manifest.toml"),
    ),
    (
        "__init__.py",
        include_str!("../addon/omnix_blender/__init__.py"),
    ),
    ("core.py", include_str!("../addon/omnix_blender/core.py")),
    (
        "dispatch.py",
        include_str!("../addon/omnix_blender/dispatch.py"),
    ),
    ("paths.py", include_str!("../addon/omnix_blender/paths.py")),
    (
        "protocol.py",
        include_str!("../addon/omnix_blender/protocol.py"),
    ),
    (
        "server.py",
        include_str!("../addon/omnix_blender/server.py"),
    ),
    (
        "handlers/__init__.py",
        include_str!("../addon/omnix_blender/handlers/__init__.py"),
    ),
    (
        "handlers/common.py",
        include_str!("../addon/omnix_blender/handlers/common.py"),
    ),
    (
        "handlers/pyexec.py",
        include_str!("../addon/omnix_blender/handlers/pyexec.py"),
    ),
    (
        "handlers/scene.py",
        include_str!("../addon/omnix_blender/handlers/scene.py"),
    ),
];

/// Find the Blender executable: `OMNIX_BLENDER_BIN`, then platform defaults, then `blender` on PATH.
pub fn find_blender() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OMNIX_BLENDER_BIN").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let mut candidates: Vec<PathBuf> = vec![
        "/Applications/Blender.app/Contents/MacOS/Blender".into(),
        "/usr/bin/blender".into(),
        "/usr/local/bin/blender".into(),
        "/snap/bin/blender".into(),
        "/opt/blender/blender".into(),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join("blender")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn blender_version(bin: &Path) -> Result<String> {
    let out = Command::new(bin)
        .arg("--version")
        .output()
        .with_context(|| format!("running {}", bin.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or("").trim().to_string())
}

/// Build the extension zip (manifest at the archive root, as `blender --command extension build` does).
pub fn build_addon_zip() -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in ADDON_FILES {
            zip.start_file(*name, opts)?;
            zip.write_all(content.as_bytes())?;
        }
        zip.finish()?;
    }
    Ok(buf.into_inner())
}

fn run_extension(bin: &Path, args: &[&str]) -> Result<(String, bool)> {
    let out = Command::new(bin)
        .args([
            "--background",
            "--factory-startup",
            "--command",
            "extension",
        ])
        .args(args)
        .output()
        .with_context(|| format!("running {}", bin.display()))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((text, out.status.success()))
}

/// Install (or reinstall) the bundled add-on into the user repository and enable it. Blender 5.3's
/// `install-file` exits 0 even on failure, so success is judged from its `STATUS` line (see docs/FINDINGS.md).
pub fn install_addon(bin: &Path) -> Result<String> {
    let dir = tempfile::tempdir().context("creating temp dir")?;
    let zip_path = dir
        .path()
        .join(format!("{ADDON_ID}-{}.zip", env!("CARGO_PKG_VERSION")));
    std::fs::write(&zip_path, build_addon_zip()?)?;
    let zip_str = zip_path.to_str().context("temp path not UTF-8")?;
    let (text, _) = run_extension(bin, &["install-file", "-r", "user_default", "-e", zip_str])?;
    if text.lines().any(|l| l.starts_with("ERROR")) {
        bail!("Blender reported an error during install:\n{text}");
    }
    if let Some(l) = text
        .lines()
        .find(|l| l.starts_with("STATUS Installed") || l.starts_with("STATUS Reinstalled"))
    {
        return Ok(l.trim_start_matches("STATUS ").to_string());
    }
    bail!("Blender did not confirm the install:\n{text}")
}

/// Disable and remove the add-on. Idempotent: not installed counts as success.
pub fn uninstall_addon(bin: &Path) -> Result<String> {
    let (text, ok) = run_extension(bin, &["remove", ADDON_ID])?;
    if text.lines().any(|l| l.starts_with("STATUS Removed")) {
        return Ok(format!("Removed \"{ADDON_ID}\""));
    }
    if text.contains("not installed") {
        return Ok(format!("\"{ADDON_ID}\" was not installed"));
    }
    if ok {
        return Ok("done".into());
    }
    bail!("Blender did not confirm the removal:\n{text}")
}

pub fn addon_installed(bin: &Path) -> Result<bool> {
    let (text, _) = run_extension(bin, &["list"])?;
    Ok(text
        .lines()
        .any(|l| l.contains(ADDON_ID) && l.contains("[installed]")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addon_files_match_disk() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("addon/omnix_blender");
        let mut on_disk = Vec::new();
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    if p.file_name().unwrap() != "__pycache__" {
                        walk(&p, base, out);
                    }
                } else if p.extension().is_some_and(|x| x == "py" || x == "toml") {
                    out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
                }
            }
        }
        walk(&root, &root, &mut on_disk);
        on_disk.sort();
        let mut embedded: Vec<String> = ADDON_FILES.iter().map(|(n, _)| n.to_string()).collect();
        embedded.sort();
        assert_eq!(on_disk, embedded, "update ADDON_FILES in src/blender.rs");
    }

    #[test]
    fn zip_has_manifest_at_root() {
        let bytes = build_addon_zip().unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(z.by_name("blender_manifest.toml").is_ok());
        assert!(z.by_name("handlers/scene.py").is_ok());
    }
}
