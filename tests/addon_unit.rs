//! Runs tests/support/unit_addon.py inside a real (headless, isolated) Blender and reports each result.
mod support;
use std::process::Command;

#[test]
fn addon_unit_tests_pass_inside_blender() {
    let bin = require_blender!();
    let dir = support::short_tempdir();
    let out = Command::new(&bin)
        .args(["--background", "--factory-startup", "--python"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/unit_addon.py"))
        .env("BLENDER_USER_RESOURCES", dir.path().join("res"))
        .env("BLENDER_USER_SCRIPTS", dir.path().join("scripts"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("UNIT_")).collect();
    let failed: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("UNIT_FAIL"))
        .collect();
    assert!(
        lines.iter().any(|l| l.starts_with("UNIT_DONE")),
        "script did not finish:\n{text}"
    );
    let passed = lines.iter().filter(|l| l.starts_with("UNIT_PASS")).count();
    assert!(
        passed >= 10,
        "suspiciously few tests ran ({passed}):\n{text}"
    );
    assert!(
        failed.is_empty(),
        "failing add-on unit tests:\n{}",
        failed.iter().map(|l| **l).collect::<Vec<_>>().join("\n")
    );
    assert!(out.status.success(), "{text}");
}
