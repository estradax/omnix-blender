//! M1 end to end: MCP client -> built binary -> Unix socket -> real headless Blender.
mod support;
use serde_json::json;
use support::*;

#[tokio::test]
async fn scene_info_and_list_objects_see_the_real_scene() {
    let bin = require_blender!();
    let mut h = Harness::start(&bin, true);
    let c = h.client().await;

    let info = ok(&c, "scene_info", json!({})).await;
    // The factory startup scene has Camera, Cube, Light.
    assert_eq!(info["counts"]["objects"], 3);
    assert_eq!(info["active_object"], "Cube");
    assert_eq!(info["blender"], "5.3.0");
    // Cross-check against bpy directly through an independent channel.
    let n = h.inspect("result = len(bpy.context.scene.objects)").await;
    assert_eq!(n, 3);

    let l = ok(&c, "list_objects", json!({"limit": 2})).await;
    assert_eq!(l["total"], 3);
    assert_eq!(l["truncated"], true);
    assert_eq!(l["objects"].as_array().unwrap().len(), 2);
    let l = ok(&c, "list_objects", json!({"type": "MESH"})).await;
    assert_eq!(l["objects"][0]["name"], "Cube");
    let l = ok(&c, "list_objects", json!({"name_glob": "Ca*"})).await;
    assert_eq!(l["total"], 1);

    err(
        &c,
        "list_objects",
        json!({"collection": "Nope"}),
        "not_found",
    )
    .await;
    let _ = c.cancel().await;
    h.shutdown().await;
}
