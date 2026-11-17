//! omnix integration (read-only use of a locally built omnix): the `read_only` policy must expose only the
//! query tools, and `deny` must be able to block run_python. Skips when omnix is not built or Blender is missing.
mod support;
use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::{Value, json};
use std::path::PathBuf;
use support::*;

fn omnix_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OMNIX_BIN") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("project/omnix/target/debug/omnix")).filter(|p| p.is_file())
}

async fn gateway(
    omnix: &std::path::Path,
    sock: &std::path::Path,
    policy: Value,
) -> (Client, tempfile::TempDir) {
    let dir = short_tempdir();
    let cfg = json!({
        "servers": {"blender": {
            "command": env!("CARGO_BIN_EXE_omnix-blender"),
            "args": ["serve"],
            "env": {"OMNIX_BLENDER_SOCKET": sock.to_str().unwrap()}
        }},
        "policy": policy
    });
    std::fs::write(dir.path().join("omnix.json"), cfg.to_string()).unwrap();
    let mut c = tokio::process::Command::new(omnix);
    c.arg("serve")
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("OMNIX_CONFIG_DIR", dir.path().join("cfg"))
        .env("OMNIX_CATALOG_CACHE", "off");
    let svc = ().serve(TokioChildProcess::new(c).unwrap()).await.unwrap();
    (svc, dir)
}

async fn text(svc: &Client, tool: &'static str, args: Value) -> String {
    let r = svc
        .call_tool(
            CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    serde_json::to_value(&r).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn omnix_read_only_policy_exposes_only_query_tools() {
    let bin = require_blender!();
    let Some(omnix) = omnix_bin() else {
        eprintln!("SKIP: omnix binary not found (build ~/project/omnix or set OMNIX_BIN)");
        return;
    };
    let mut h = Harness::start(&bin, true);
    let (svc, _d) = gateway(&omnix, &h.sock, json!({"read_only": true})).await;

    // Search lists readable tools only.
    let found = text(&svc, "search", json!({"query": "blender", "limit": 100})).await;
    for ro in [
        "scene_info",
        "list_objects",
        "get_object",
        "viewport_screenshot",
        "blender_guide",
    ] {
        assert!(
            found.contains(&format!("blender.{ro}")),
            "{ro} should be visible:\n{found}"
        );
    }
    for w in [
        "create_primitive",
        "delete_objects",
        "run_python",
        "set_transform",
        "render_image",
        "open_file",
        "undo",
    ] {
        assert!(
            !found.contains(&format!("blender.{w}")),
            "{w} must be hidden under read_only:\n{found}"
        );
    }

    // A query works end to end through the gateway and real Blender.
    let out = text(
        &svc,
        "execute",
        json!({"code": "return await blender.scene_info({})"}),
    )
    .await;
    assert!(out.contains("\"objects\":3"), "{out}");

    // A write is refused by policy and Blender is untouched.
    let out = text(
        &svc,
        "execute",
        json!({"code": "return await blender.create_primitive({type:'cube', name:'Nope'})"}),
    )
    .await;
    assert!(out.contains("error:") && out.contains("policy"), "{out}");
    let out = text(
        &svc,
        "execute",
        json!({"code": "return await blender.run_python({code:'result=1'})"}),
    )
    .await;
    assert!(out.contains("error:") && out.contains("policy"), "{out}");
    assert_eq!(h.inspect("result='Nope' in bpy.data.objects").await, false);
    let _ = svc.cancel().await;
    h.shutdown().await;
}

#[tokio::test]
async fn omnix_deny_blocks_run_python_but_not_the_rest() {
    let bin = require_blender!();
    let Some(omnix) = omnix_bin() else {
        eprintln!("SKIP: omnix binary not found (build ~/project/omnix or set OMNIX_BIN)");
        return;
    };
    let mut h = Harness::start(&bin, true);
    let (svc, _d) = gateway(&omnix, &h.sock, json!({"deny": ["blender.run_python"]})).await;
    let out = text(
        &svc,
        "execute",
        json!({"code": "return await blender.run_python({code:'result=1'})"}),
    )
    .await;
    assert!(out.contains("denied by policy"), "{out}");
    let out = text(
        &svc,
        "execute",
        json!({"code": "return await blender.create_primitive({type:'cube', name:'Allowed'})"}),
    )
    .await;
    assert!(out.contains("Allowed"), "{out}");
    assert_eq!(
        h.inspect("result='Allowed' in bpy.data.objects").await,
        true
    );
    let _ = svc.cancel().await;
    h.shutdown().await;
}
