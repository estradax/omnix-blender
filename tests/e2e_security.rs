//! M3 end to end: security, lifecycle, install, reconnect and robustness against a real headless Blender.
mod support;
use omnix_blender::{error::LinkError, link::Link};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::time::Duration;
use support::*;

fn mode(p: &std::path::Path) -> u32 {
    std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn run_python_is_disabled_by_default_with_instructions() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, false);
    let c = h.client().await;
    let t = err(&c, "run_python", json!({"code": "result = 1"}), "disabled").await;
    assert!(
        t.contains("Allow run_python") && t.contains("Preferences"),
        "{t}"
    );
    // Nothing ran: a side effect would be visible.
    let link = Link::new(&h.sock);
    let r = link
        .call("status", json!({}), Duration::from_secs(5), true)
        .await
        .unwrap();
    assert_eq!(r["run_python_enabled"], false);
    let e = link
        .call(
            "run_python",
            json!({"code": "bpy.data.objects.new('Pwned', None)"}),
            Duration::from_secs(5),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(e.code(), "disabled");
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn socket_directory_and_file_are_private() {
    let bin = require_blender!();
    let mut h = Harness::unstarted(&bin, false);
    h.sock = h.dir.path().join("fresh").join("b.sock"); // directory must be created by the add-on
    h.spawn();
    assert_eq!(
        mode(h.sock.parent().unwrap()),
        0o700,
        "created directory mode"
    );
    assert_eq!(mode(&h.sock), 0o600, "socket mode");
    h.shutdown().await;
    assert!(!h.sock.exists(), "socket file removed on clean shutdown");
}

#[tokio::test]
async fn refuses_a_group_or_world_writable_socket_directory() {
    let bin = require_blender!();
    let mut h = Harness::unstarted(&bin, false);
    let open = h.dir.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    h.sock = open.join("b.sock");
    let e = h.try_spawn().unwrap_err();
    assert!(e.contains("must be private"), "{e}");
    assert!(!h.sock.exists());
    // The client side refuses such a socket too.
    let link = Link::new(&h.sock);
    assert!(matches!(link.hello().await, Err(LinkError::BadPath(_))));
}

#[tokio::test]
async fn client_refuses_an_untrusted_socket_directory() {
    let d = short_tempdir();
    let open = d.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let f = support::fake::Fake::start(
        &open.join("f.sock"),
        std::sync::Arc::new(|_, _| support::fake::Reply::Ok(json!({}))),
    );
    let e = Link::new(&f.path).hello().await.unwrap_err();
    assert!(matches!(e, LinkError::BadPath(_)), "{e:?}");
    assert!(e.to_string().contains("accessible by group/others"));
}

#[tokio::test]
async fn refuses_to_replace_a_regular_file() {
    let bin = require_blender!();
    let mut h = Harness::unstarted(&bin, false);
    std::fs::write(&h.sock, b"precious").unwrap();
    let e = h.try_spawn().unwrap_err();
    assert!(e.contains("not a socket"), "{e}");
    assert_eq!(std::fs::read(&h.sock).unwrap(), b"precious");
}

#[tokio::test]
async fn stale_socket_from_a_crash_is_replaced() {
    let bin = require_blender!();
    let mut h = Harness::unstarted(&bin, false);
    drop(std::os::unix::net::UnixListener::bind(&h.sock).unwrap()); // leaves a dead socket file behind
    assert!(h.sock.exists());
    h.spawn();
    let c = h.client().await;
    ok(&c, "scene_info", json!({})).await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn a_live_socket_is_not_hijacked_by_a_second_blender() {
    let bin = require_blender!();
    let mut a = Harness::start(&bin, false);
    let mut b = Harness::unstarted(&bin, false);
    b.sock = a.sock.clone();
    let e = b.try_spawn().unwrap_err();
    assert!(e.contains("already listening"), "{e}");
    let c = a.client().await;
    ok(&c, "scene_info", json!({})).await; // the first one is untouched
    let _ = c.cancel().await;
    a.shutdown().await;
}

fn roundtrip(s: &mut UnixStream, line: &[u8]) -> Value {
    s.write_all(line).unwrap();
    s.write_all(b"\n").unwrap();
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut resp = String::new();
    r.read_line(&mut resp).unwrap();
    serde_json::from_str(&resp).unwrap_or_else(|e| panic!("bad response {resp:?}: {e}"))
}

#[tokio::test]
async fn malformed_and_oversized_lines_do_not_crash_the_addon() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, false);
    let hello = br#"{"v":1,"id":1,"method":"hello","params":{}}"#;

    // Before hello nothing executes and the connection is closed.
    let mut s = UnixStream::connect(&h.sock).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let r = roundtrip(
        &mut s,
        br#"{"v":1,"id":9,"method":"scene_info","params":{}}"#,
    );
    assert_eq!(r["error"]["code"], "protocol_error");

    // After hello, garbage is answered with an error and the connection survives.
    let mut s = UnixStream::connect(&h.sock).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    assert_eq!(roundtrip(&mut s, hello)["ok"], true);
    assert_eq!(
        roundtrip(&mut s, b"this is not json")["error"]["code"],
        "protocol_error"
    );
    assert_eq!(
        roundtrip(&mut s, b"[1,2,3]")["error"]["code"],
        "protocol_error"
    );
    assert_eq!(
        roundtrip(&mut s, &[0xff, 0xfe, 0xfd])["error"]["code"],
        "protocol_error"
    );
    assert_eq!(
        roundtrip(&mut s, br#"{"v":1,"id":2,"method":5}"#)["error"]["code"],
        "protocol_error"
    );
    assert_eq!(
        roundtrip(
            &mut s,
            br#"{"v":1,"id":3,"method":"scene_info","params":[1]}"#
        )["error"]["code"],
        "invalid_argument"
    );
    assert_eq!(
        roundtrip(
            &mut s,
            br#"{"v":1,"id":4,"method":"__import__('os')","params":{}}"#
        )["error"]["code"],
        "unsupported"
    );
    let r = roundtrip(
        &mut s,
        br#"{"v":1,"id":5,"method":"scene_info","params":{}}"#,
    );
    assert_eq!(r["ok"], true, "same connection still works: {r}");

    // Wrong protocol version is refused and closes the connection.
    let mut s = UnixStream::connect(&h.sock).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let r = roundtrip(&mut s, br#"{"v":2,"id":1,"method":"hello","params":{}}"#);
    assert_eq!(r["error"]["code"], "unsupported");

    // An endless line (17 MiB, no newline) is rejected and closed, memory is bounded.
    let mut s = UnixStream::connect(&h.sock).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    assert_eq!(roundtrip(&mut s, hello)["ok"], true);
    let chunk = vec![b'a'; 1024 * 1024];
    let mut closed = false;
    for _ in 0..20 {
        if s.write_all(&chunk).is_err() {
            closed = true;
            break;
        }
    }
    let mut resp = String::new();
    let _ = BufReader::new(s.try_clone().unwrap()).read_line(&mut resp);
    assert!(
        closed || resp.contains("exceeds 16 MiB"),
        "oversized line not rejected: {resp:?}"
    );
    assert!(resp.contains("invalid_argument"), "{resp:?}");
    let mut rest = Vec::new();
    let _ = s.read_to_end(&mut rest); // connection is closed by the add-on

    // The add-on is still alive and serving.
    let c = h.client().await;
    ok(&c, "scene_info", json!({})).await;
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn the_addon_binds_no_tcp_port() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, false);
    let pid = h.pid().to_string();
    let all = std::process::Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid])
        .output();
    let Ok(all) = all else {
        eprintln!("SKIP: lsof not available");
        return;
    };
    let all = String::from_utf8_lossy(&all.stdout).to_string();
    assert!(
        all.contains("b.sock"),
        "lsof must see the unix socket (else this check is vacuous):\n{all}"
    );
    let tcp = std::process::Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid, "-i", "-sTCP:LISTEN"])
        .output()
        .unwrap();
    let tcp = String::from_utf8_lossy(&tcp.stdout);
    assert!(tcp.trim().is_empty(), "Blender listens on TCP:\n{tcp}");
    let udp = std::process::Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid, "-iUDP"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&udp.stdout).trim().is_empty());
    h.shutdown().await;
}

#[tokio::test]
async fn reconnects_after_blender_crashes_and_restarts() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, false);
    let c = h.client().await;
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "Before"}),
    )
    .await;
    h.crash(); // SIGKILL: leaves a stale socket file
    assert!(h.sock.exists());
    let t = std::time::Instant::now();
    err(&c, "scene_info", json!({}), "not_connected").await;
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "must fail fast, not hang"
    );
    h.spawn(); // replaces the stale socket
    // The same MCP session keeps working, including for a mutating call.
    let r = ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "After"}),
    )
    .await;
    assert_eq!(r["name"], "After");
    let l = ok(&c, "list_objects", json!({"name_glob": "Before"})).await;
    assert_eq!(l["total"], 0, "a new Blender process has a fresh scene");
    let _ = c.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn long_requests_time_out_and_others_get_busy() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let slow = Link::new(&h.sock);
    let other = Link::new(&h.sock);
    other.hello().await.unwrap();
    let t = tokio::spawn(async move {
        slow.call(
            "run_python",
            json!({"code": "import time; time.sleep(3)", "undo": false}),
            Duration::from_secs(60),
            false,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(800)).await;
    let e = other
        .call("scene_info", json!({}), Duration::from_secs(5), true)
        .await
        .unwrap_err();
    assert_eq!(e.code(), "busy", "{e}");
    assert!(t.await.unwrap().is_ok());
    other
        .call("scene_info", json!({}), Duration::from_secs(5), true)
        .await
        .unwrap();

    // A request that exceeds its budget reports `timeout`; the add-on is busy until it really finishes.
    let l = Link::new(&h.sock);
    let e = l
        .call(
            "run_python",
            json!({"code": "import time; time.sleep(3)", "undo": false}),
            Duration::from_secs(1),
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(e.code(), "timeout", "{e}");
    assert!(e.to_string().contains("still running"), "{e}");
    let e = l
        .call("scene_info", json!({}), Duration::from_secs(5), true)
        .await
        .unwrap_err();
    assert_eq!(e.code(), "busy");
    tokio::time::sleep(Duration::from_secs(3)).await;
    l.call("scene_info", json!({}), Duration::from_secs(5), true)
        .await
        .unwrap();
    h.shutdown().await;
}

fn installed(env: &ExtEnv) -> bool {
    env.dir
        .path()
        .join("res/extensions/user_default/omnix_blender/blender_manifest.toml")
        .exists()
}

fn stdout(o: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[tokio::test]
async fn install_addon_is_idempotent_and_reversible_in_an_isolated_profile() {
    let bin = require_blender!();
    let env = ExtEnv::new(&bin);
    assert!(!installed(&env));
    let o = env.omnix(&["install-addon"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert!(stdout(&o).contains("Installed"), "{}", stdout(&o));
    assert!(installed(&env));
    let o = env.omnix(&["install-addon"]);
    assert!(
        o.status.success() && stdout(&o).contains("Reinstalled"),
        "{}",
        stdout(&o)
    );
    let init = std::fs::read_to_string(
        env.dir
            .path()
            .join("res/extensions/user_default/omnix_blender/handlers/objects.py"),
    )
    .unwrap();
    assert!(
        init.contains("create_primitive"),
        "installed files are the embedded ones"
    );

    let o = env.omnix(&["install-addon", "--uninstall"]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert!(!installed(&env), "files removed");
    let o = env.omnix(&["install-addon", "--uninstall"]);
    assert!(
        o.status.success(),
        "uninstall is idempotent: {}",
        stdout(&o)
    );
    assert!(stdout(&o).contains("not installed"));
}

#[tokio::test]
async fn install_addon_fails_clearly_without_blender() {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_omnix-blender"));
    c.arg("install-addon")
        .env("OMNIX_BLENDER_BIN", "/nonexistent/blender");
    let o = c.output().unwrap();
    assert!(!o.status.success());
    assert!(stdout(&o).contains("Blender not found"), "{}", stdout(&o));
}

#[tokio::test]
async fn extension_lifecycle_enable_start_stop_disable_enable() {
    let bin = require_blender!();
    let env = ExtEnv::new(&bin);
    assert!(env.omnix(&["install-addon"]).status.success());
    let mut s = env.spawn_script("ext_lifecycle.py", true, &[]);
    s.wait_for("LIFECYCLE_OK", "LIFECYCLE_FAIL", 120);
    let st = s.wait_exit(60);
    assert!(st.success(), "{}", s.text());
}

#[tokio::test]
async fn installed_extension_doctor_mcp_and_run_python_preference() {
    let bin = require_blender!();
    let env = ExtEnv::new(&bin);
    // Doctor before anything is installed or running: red, with actionable hints.
    let o = env.omnix(&["doctor"]);
    assert!(!o.status.success());
    assert!(stdout(&o).contains("install-addon"), "{}", stdout(&o));
    assert!(env.omnix(&["install-addon"]).status.success());
    let o = env.omnix(&["doctor"]);
    assert!(
        !o.status.success(),
        "installed but not started is still a problem"
    );
    assert!(stdout(&o).contains("Start"), "{}", stdout(&o));

    for allow in [false, true] {
        let mut s = env.spawn_script(
            "ext_session.py",
            true,
            &[("OMNIX_TEST_ALLOW", if allow { "1" } else { "0" })],
        );
        s.wait_for("SESSION_READY", "SESSION_FAIL", 120);
        let o = env.omnix(&["doctor"]);
        let out = stdout(&o);
        assert!(o.status.success(), "doctor should be green:\n{out}");
        assert!(
            out.contains("add-on installed") && out.contains("connected: Blender 5.3"),
            "{out}"
        );
        assert!(
            out.contains(if allow {
                "run_python is ENABLED"
            } else {
                "run_python is disabled"
            }),
            "{out}"
        );
        assert_eq!(mode(&env.sock), 0o600);

        let c = connect_client(&env.sock).await;
        ok(&c, "scene_info", json!({})).await;
        let o = call(&c, "run_python", json!({"code": "result = 6 * 7"})).await;
        if allow {
            assert!(!o.is_error, "{}", o.text);
            assert_eq!(o.json()["result"], 42);
        } else {
            assert!(o.is_error && o.text.contains("disabled"), "{}", o.text);
        }
        let _ = c.cancel().await;
        let st = s.finish();
        assert!(st.success(), "{}", s.text());
        assert!(!env.sock.exists(), "socket removed when the session stops");
    }
}

/// GUI smoke test: opens a real Blender window briefly (timer-driven pump, screenshot, undo, render).
/// Run with `cargo test --test e2e_security -- --ignored gui_smoke`.
#[tokio::test]
#[ignore = "opens a Blender GUI window"]
async fn gui_smoke() {
    let bin = require_blender!();
    let env = ExtEnv::new(&bin);
    assert!(env.omnix(&["install-addon"]).status.success());
    let folder = env.dir.path().to_str().unwrap().to_string();
    let mut s = env.spawn_script(
        "ext_session.py",
        false,
        &[
            ("OMNIX_TEST_ALLOW", "0"),
            ("OMNIX_TEST_FOLDERS", folder.as_str()),
        ],
    );
    s.wait_for("SESSION_READY", "SESSION_FAIL", 120);
    let c = connect_client(&env.sock).await;
    let info = ok(&c, "scene_info", json!({})).await;
    assert_eq!(info["counts"]["objects"], 3);
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "GuiBox"}),
    )
    .await;
    ok(&c, "undo", json!({})).await;
    assert_eq!(
        ok(&c, "list_objects", json!({"name_glob": "GuiBox"})).await["total"],
        0,
        "undo from a GUI timer works"
    );
    ok(&c, "redo", json!({})).await;
    assert_eq!(
        ok(&c, "list_objects", json!({"name_glob": "GuiBox"})).await["total"],
        1
    );
    // Operators that need a context (apply modifier, import/export, save) from a GUI timer.
    ok(
        &c,
        "add_modifier",
        json!({"object": "GuiBox", "type": "SUBSURF", "properties": {"levels": 1}}),
    )
    .await;
    ok(
        &c,
        "apply_modifier",
        json!({"object": "GuiBox", "name": "Subsurf"}),
    )
    .await;
    assert!(
        ok(&c, "get_object", json!({"name": "GuiBox"})).await["mesh"]["vertices"]
            .as_u64()
            .unwrap()
            > 8
    );
    let obj = env.dir.path().join("gui.obj");
    ok(&c, "export_file", json!({"path": obj.to_str().unwrap()})).await;
    assert!(
        ok(&c, "import_file", json!({"path": obj.to_str().unwrap()})).await["count"]
            .as_u64()
            .unwrap()
            >= 1
    );
    ok(
        &c,
        "save_file",
        json!({"path": env.dir.path().join("gui.blend").to_str().unwrap()}),
    )
    .await;
    assert_eq!(ok(&c, "scene_info", json!({})).await["unsaved"], false);
    let shot = call(&c, "viewport_screenshot", json!({"max_edge": 400})).await;
    assert!(!shot.is_error, "{}", shot.text);
    assert!(shot.raw.content.iter().any(|b| b.as_image().is_some()));
    ok(
        &c,
        "set_render_settings",
        json!({"engine": "BLENDER_WORKBENCH", "resolution_x": 200, "resolution_y": 100}),
    )
    .await;
    let r = call(&c, "render_image", json!({})).await;
    assert!(!r.is_error, "{}", r.text);
    let _ = c.cancel().await;
    let st = s.finish();
    assert!(st.success(), "{}", s.text());
}
