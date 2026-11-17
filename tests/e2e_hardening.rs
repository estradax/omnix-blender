//! Regression tests for the review findings (each was written to fail against the previous behaviour).
mod support;
use serde_json::json;
use support::*;

/// open_file must never run scripts embedded in the file, even when the profile trusts auto-execution.
#[tokio::test]
async fn open_file_never_auto_executes_embedded_scripts() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let c = h.client().await;
    let marker = h.dir.path().join("marker.txt");
    let blend = h.dir.path().join("trap.blend");
    h.inspect(&format!(
        "t = bpy.data.texts.new('autorun.py')\nt.write(\"open(r'{}', 'w').write('pwned')\")\nt.use_module = True\nbpy.ops.wm.save_as_mainfile(filepath=r'{}')",
        marker.display(),
        blend.display()
    ))
    .await;
    assert!(!marker.exists(), "saving must not run the script");
    // The profile trusts scripts (the dangerous configuration).
    h.inspect("bpy.context.preferences.filepaths.use_scripts_auto_execute = True")
        .await;
    ok(
        &c,
        "open_file",
        json!({"path": blend.to_str().unwrap(), "discard_changes": true}),
    )
    .await;
    // The text block was loaded (so the file really was opened) but never executed.
    assert_eq!(
        h.inspect("result = 'autorun.py' in bpy.data.texts").await,
        true
    );
    assert!(!marker.exists(), "embedded script ran during open_file");
    let _ = c.cancel().await;
    h.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// Path policy (save_file / export_file / import_file / open_file / set_render_settings.output_path)
// ---------------------------------------------------------------------------------------------

fn s(p: &std::path::Path) -> String {
    p.to_str().unwrap().to_string()
}

fn started(bin: &std::path::Path, run_python: bool, tweak: impl FnOnce(&mut Harness)) -> Harness {
    let mut h = Harness::unstarted(bin, run_python);
    tweak(&mut h);
    h.spawn();
    h
}

#[tokio::test]
async fn export_refuses_to_overwrite_without_overwrite_true() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let c = h.client().await;
    let target = h.dir.path().join("a.stl");
    std::fs::write(&target, b"precious").unwrap();
    err(
        &c,
        "export_file",
        json!({"path": s(&target)}),
        "overwrite=true",
    )
    .await;
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"precious",
        "file must be untouched"
    );
    let r = ok(
        &c,
        "export_file",
        json!({"path": s(&target), "overwrite": true}),
    )
    .await;
    assert!(r["bytes"].as_u64().unwrap() > 50);
    assert_ne!(std::fs::read(&target).unwrap(), b"precious");
    // save_file as well
    let blend = h.dir.path().join("keep.blend");
    std::fs::write(&blend, b"precious").unwrap();
    err(
        &c,
        "save_file",
        json!({"path": s(&blend)}),
        "overwrite=true",
    )
    .await;
    assert_eq!(std::fs::read(&blend).unwrap(), b"precious");
    ok(
        &c,
        "save_file",
        json!({"path": s(&blend), "overwrite": true}),
    )
    .await;
    assert!(std::fs::metadata(&blend).unwrap().len() > 1000);
    // Saving over the file that is currently open (the normal workflow) needs no flag.
    ok(&c, "save_file", json!({})).await;
    ok(&c, "save_file", json!({"path": s(&blend)})).await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn hidden_path_components_are_always_refused() {
    let bin = require_blender!();
    // Even with the "outside" preference ON, hidden components stay refused.
    let mut h = started(&bin, true, |h| h.allow_outside = true);
    let c = h.client().await;
    let hidden = h.dir.path().join(".hidden");
    std::fs::create_dir(&hidden).unwrap();
    let t = err(
        &c,
        "export_file",
        json!({"path": s(&hidden.join("x.obj"))}),
        "hidden",
    )
    .await;
    assert!(t.contains("disabled"), "{t}");
    assert!(!hidden.join("x.obj").exists());
    err(
        &c,
        "export_file",
        json!({"path": s(&h.dir.path().join(".x.obj"))}),
        "hidden",
    )
    .await;
    err(
        &c,
        "save_file",
        json!({"path": s(&hidden.join("x.blend"))}),
        "hidden",
    )
    .await;
    err(
        &c,
        "set_render_settings",
        json!({"output_path": s(&hidden.join("r_####"))}),
        "hidden",
    )
    .await;
    assert!(!hidden.join("x.obj").exists() && !hidden.join("x.blend").exists());
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn symlinks_into_protected_locations_are_refused_even_with_outside_access_on() {
    let bin = require_blender!();
    let mut h = started(&bin, true, |h| h.allow_outside = true);
    let c = h.client().await;
    let launch = h.home.join("Library/LaunchAgents");
    // File symlink -> protected dir; directory symlink -> ~/.ssh.
    let link = h.dir.path().join("evil.obj");
    std::os::unix::fs::symlink(launch.join("evil.obj"), &link).unwrap();
    let dirlink = h.dir.path().join("sshlink");
    std::os::unix::fs::symlink(h.home.join(".ssh"), &dirlink).unwrap();
    err(
        &c,
        "export_file",
        json!({"path": s(&link), "overwrite": true}),
        "protected",
    )
    .await;
    err(
        &c,
        "export_file",
        json!({"path": s(&dirlink.join("id.obj"))}),
        "hidden",
    )
    .await;
    assert!(
        !launch.join("evil.obj").exists(),
        "nothing written through the symlink"
    );
    assert_eq!(std::fs::read_dir(h.home.join(".ssh")).unwrap().count(), 0);
    // A direct (non-hidden) protected path is refused as well.
    err(
        &c,
        "export_file",
        json!({"path": s(&launch.join("x.obj"))}),
        "protected",
    )
    .await;
    err(
        &c,
        "save_file",
        json!({"path": s(&launch.join("x.blend"))}),
        "protected",
    )
    .await;
    // And reading: opening/importing from protected places.
    err(
        &c,
        "import_file",
        json!({"path": s(&launch.join("x.obj"))}),
        "protected",
    )
    .await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn paths_outside_the_allowed_roots_need_the_preference() {
    let bin = require_blender!();
    let outside = short_tempdir();
    let target = outside.path().join("o.obj");
    let blend = outside.path().join("o.blend");

    let mut h = Harness::start(&bin, true);
    let c = h.client().await;
    let t = err(&c, "export_file", json!({"path": s(&target)}), "disabled").await;
    assert!(
        t.contains("Allowed folders") && t.contains("Allow file access outside"),
        "message must say how to allow: {t}"
    );
    assert!(!target.exists());
    err(&c, "save_file", json!({"path": s(&blend)}), "disabled").await;
    err(
        &c,
        "set_render_settings",
        json!({"output_path": s(&outside.path().join("r_####"))}),
        "disabled",
    )
    .await;
    std::fs::write(&target, b"v 0 0 0\n").unwrap();
    err(&c, "import_file", json!({"path": s(&target)}), "disabled").await;
    err(
        &c,
        "open_file",
        json!({"path": s(&blend), "discard_changes": true}),
        "disabled",
    )
    .await;
    // Relative paths are not accepted at all.
    err(&c, "export_file", json!({"path": "x.obj"}), "absolute").await;
    std::fs::remove_file(&target).unwrap();
    let _ = c.cancel().await;
    h.shutdown().await;

    let mut h = started(&bin, true, |h| h.allow_outside = true);
    let c = h.client().await;
    ok(&c, "export_file", json!({"path": s(&target)})).await;
    assert!(target.exists());
    ok(&c, "save_file", json!({"path": s(&blend)})).await;
    ok(
        &c,
        "set_render_settings",
        json!({"output_path": s(&outside.path().join("r_####"))}),
    )
    .await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn the_folder_of_the_saved_blend_is_an_allowed_root() {
    let bin = require_blender!();
    let work = short_tempdir();
    let other = short_tempdir();
    let mut h = started(&bin, true, |h| h.allowed_folders = vec![]); // no allowed folders configured
    let c = h.client().await;
    err(
        &c,
        "export_file",
        json!({"path": s(&work.path().join("a.obj"))}),
        "disabled",
    )
    .await;
    // The scene gets saved into `work` (run_python is outside the file policy by design).
    h.inspect(&format!(
        "bpy.ops.wm.save_as_mainfile(filepath=r'{}')",
        work.path().join("s.blend").display()
    ))
    .await;
    ok(
        &c,
        "export_file",
        json!({"path": s(&work.path().join("a.obj"))}),
    )
    .await;
    assert!(work.path().join("a.obj").exists());
    err(
        &c,
        "export_file",
        json!({"path": s(&other.path().join("a.obj"))}),
        "disabled",
    )
    .await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn extension_and_format_must_agree() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let c = h.client().await;
    let d = h.dir.path();
    err(
        &c,
        "export_file",
        json!({"path": s(&d.join("x.obj")), "format": "fbx"}),
        "contradicts",
    )
    .await;
    err(
        &c,
        "export_file",
        json!({"path": s(&d.join("x.sh")), "format": "obj"}),
        "unsupported file extension",
    )
    .await;
    err(
        &c,
        "export_file",
        json!({"path": s(&d.join("noext")), "format": "obj"}),
        "unsupported file extension",
    )
    .await;
    err(
        &c,
        "export_file",
        json!({"path": s(&d.join("x.gltf")), "format": "obj"}),
        "contradicts",
    )
    .await;
    assert!(!d.join("x.obj").exists() && !d.join("x.sh").exists() && !d.join("noext").exists());
    ok(
        &c,
        "export_file",
        json!({"path": s(&d.join("y.glb")), "format": "gltf"}),
    )
    .await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn side_files_are_reported_and_checked() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let c = h.client().await;
    let d = h.dir.path().canonicalize().unwrap();
    let r = ok(&c, "export_file", json!({"path": s(&d.join("m.obj"))})).await;
    let side: Vec<&str> = r["side_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        side,
        [s(&d.join("m.mtl")).as_str()],
        "the .mtl must be reported: {r}"
    );
    assert!(d.join("m.mtl").exists());
    // Re-export: the existing .mtl is protected too, even if the .obj is gone.
    std::fs::remove_file(d.join("m.obj")).unwrap();
    err(
        &c,
        "export_file",
        json!({"path": s(&d.join("m.obj"))}),
        "side file",
    )
    .await;
    assert!(!d.join("m.obj").exists());
    // A symlinked .mtl pointing into a protected location is refused before anything is written.
    std::fs::remove_file(d.join("m.mtl")).unwrap();
    std::os::unix::fs::symlink(h.home.join("Library/LaunchAgents/m.mtl"), d.join("m.mtl")).unwrap();
    err(
        &c,
        "export_file",
        json!({"path": s(&d.join("m.obj")), "overwrite": true}),
        "protected",
    )
    .await;
    assert!(!h.home.join("Library/LaunchAgents/m.mtl").exists());
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn normal_workflow_inside_the_allowed_folder_still_works() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let c = h.client().await;
    let d = h.dir.path().join("proj");
    std::fs::create_dir(&d).unwrap();
    ok(&c, "save_file", json!({"path": s(&d.join("scene.blend"))})).await;
    for ext in ["obj", "stl", "ply", "usdc", "abc", "fbx", "glb"] {
        let p = d.join(format!("e.{ext}"));
        ok(&c, "export_file", json!({"path": s(&p)})).await;
        assert!(p.exists(), "{ext}");
    }
    ok(
        &c,
        "set_render_settings",
        json!({"output_path": s(&d.join("frame_####"))}),
    )
    .await;
    ok(
        &c,
        "open_file",
        json!({"path": s(&d.join("scene.blend")), "discard_changes": true}),
    )
    .await;
    ok(&c, "import_file", json!({"path": s(&d.join("e.obj"))})).await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn file_policy_preferences_work_through_the_installed_extension() {
    let bin = require_blender!();
    let env = ExtEnv::new(&bin);
    assert!(env.omnix(&["install-addon"]).status.success());
    let out_dir = short_tempdir();
    let target = out_dir.path().join("p.obj");
    let cases: [(&[(&str, &str)], bool); 3] = [
        (&[("OMNIX_TEST_ALLOW", "0")], false),
        (
            &[
                ("OMNIX_TEST_ALLOW", "0"),
                ("OMNIX_TEST_FOLDERS", out_dir.path().to_str().unwrap()),
            ],
            true,
        ),
        (
            &[("OMNIX_TEST_ALLOW", "0"), ("OMNIX_TEST_ALLOW_OUTSIDE", "1")],
            true,
        ),
    ];
    for (extra, expect_ok) in cases {
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(out_dir.path().join("p.mtl"));
        let mut s = env.spawn_script("ext_session.py", true, extra);
        s.wait_for("SESSION_READY", "SESSION_FAIL", 120);
        let c = connect_client(&env.sock).await;
        let o = call(&c, "export_file", json!({"path": s_(&target)})).await;
        assert_eq!(!o.is_error, expect_ok, "{extra:?}: {}", o.text);
        assert_eq!(target.exists(), expect_ok);
        if !expect_ok {
            assert!(o.text.contains("disabled"), "{}", o.text);
        }
        let _ = c.cancel().await;
        assert!(s.finish().success(), "{}", s.text());
    }
}

fn s_(p: &std::path::Path) -> String {
    p.to_str().unwrap().to_string()
}
