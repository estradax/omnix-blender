//! Behaviour with no Blender at all (no real Blender needed).
mod support;
use serde_json::json;
use support::*;

#[tokio::test]
async fn handshake_and_tool_list_work_without_blender_and_calls_fail_fast() {
    let dir = short_tempdir();
    let sock = dir.path().join("none.sock");
    let c = connect_client(&sock).await;
    let tools = c.list_tools(None).await.unwrap().tools;
    assert!(tools.iter().any(|t| t.name == "scene_info"));
    let t = std::time::Instant::now();
    let text = err(&c, "scene_info", json!({}), "not_connected").await;
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "must not hang"
    );
    assert!(
        text.contains("N panel"),
        "message must say how to fix it: {text}"
    );
    let _ = c.cancel().await;
}
