//! `omnix-blender doctor`: check Blender, the add-on and the socket; print what is wrong and how to fix it.

use crate::{blender, link::Link, paths};

pub struct Report {
    pub lines: Vec<String>,
    pub ok: bool,
}

impl Report {
    fn pass(&mut self, s: impl Into<String>) {
        self.lines.push(format!("[ok]   {}", s.into()));
    }
    fn fail(&mut self, s: impl Into<String>) {
        self.ok = false;
        self.lines.push(format!("[FAIL] {}", s.into()));
    }
    fn info(&mut self, s: impl Into<String>) {
        self.lines.push(format!("       {}", s.into()));
    }
}

pub async fn run() -> Report {
    let mut r = Report {
        lines: vec![],
        ok: true,
    };
    match blender::find_blender() {
        None => r.fail("Blender not found (looked in /Applications, /usr/bin, PATH; set OMNIX_BLENDER_BIN to override)"),
        Some(bin) => {
            match blender::blender_version(&bin) {
                Ok(v) => r.pass(format!("Blender: {} ({})", bin.display(), v)),
                Err(e) => r.fail(format!("cannot run {}: {e:#}", bin.display())),
            }
            match blender::addon_installed(&bin) {
                Ok(true) => r.pass("add-on installed in a user repository (`extension list`)"),
                Ok(false) => r.fail("add-on not installed; run `omnix-blender install-addon`"),
                Err(e) => r.fail(format!("could not list extensions: {e:#}")),
            }
        }
    }
    let path = paths::default_socket_path();
    r.info(format!("socket path: {}", path.display()));
    if let Err(e) = paths::check_len(&path) {
        r.fail(e);
        return r;
    }
    if !path.exists() {
        r.fail("socket does not exist: open Blender > N panel > Omnix > Start");
        return r;
    }
    let problems = paths::permission_problems(&path);
    if problems.is_empty() {
        r.pass("socket directory/file permissions are private (dir not group/world-writable, file 600)");
    }
    for p in problems {
        r.fail(p);
    }
    let link = Link::new(&path);
    match link.hello().await {
        Ok(h) => {
            r.pass(format!(
                "connected: Blender {} / add-on {} (protocol v{}, {} methods{})",
                h.blender,
                h.addon,
                h.v,
                h.features.len(),
                if h.headless { ", headless" } else { "" }
            ));
            match link
                .call(
                    "status",
                    serde_json::json!({}),
                    crate::link::DEFAULT_TIMEOUT,
                    true,
                )
                .await
            {
                Ok(s) => {
                    r.info(format!(
                        "file access outside allowed folders is {}",
                        if s["file_access_outside_allowed_folders"].as_bool() == Some(true) {
                            "ENABLED"
                        } else {
                            "off (default)"
                        }
                    ));
                    r.info(format!(
                        "run_python is {}",
                        if s["run_python_enabled"].as_bool() == Some(true) {
                            "ENABLED"
                        } else {
                            "disabled (default)"
                        }
                    ))
                }
                Err(e) => r.fail(format!("status call failed: {e}")),
            }
        }
        Err(e) => r.fail(format!("{e}")),
    }
    r
}
