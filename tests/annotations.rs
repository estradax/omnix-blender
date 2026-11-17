//! Tool annotations and forwarding, checked against a fake add-on (no Blender needed).
mod support;
use serde_json::json;
use std::sync::Arc;
use support::fake::{Fake, Reply};
use support::*;

/// (tool, read_only, destructive). `None` destructive = not asserted (read-only tools).
const TABLE: &[(&str, bool, Option<bool>)] = &[
    ("blender_guide", true, None),
    ("scene_info", true, None),
    ("list_objects", true, None),
    ("get_object", true, None),
    ("viewport_screenshot", true, None),
    ("create_primitive", false, Some(false)),
    ("create_object", false, Some(false)),
    ("set_transform", false, Some(false)),
    ("delete_objects", false, Some(true)),
    ("duplicate_object", false, Some(false)),
    ("set_parent", false, Some(false)),
    ("select_objects", false, Some(false)),
    ("collection", false, Some(false)),
    ("set_material", false, Some(false)),
    ("add_modifier", false, Some(false)),
    ("apply_modifier", false, Some(true)),
    ("remove_modifier", false, Some(true)),
    ("set_render_settings", false, Some(false)),
    ("render_image", false, Some(false)),
    ("save_file", false, Some(true)),
    ("open_file", false, Some(true)),
    ("new_file", false, Some(true)),
    ("import_file", false, Some(false)),
    ("export_file", false, Some(true)),
    ("undo", false, Some(false)),
    ("redo", false, Some(false)),
    ("run_python", false, Some(true)),
];

#[tokio::test]
async fn every_tool_has_the_right_annotations() {
    let d = short_tempdir();
    let c = connect_client(&d.path().join("none.sock")).await;
    let tools = c.list_tools(None).await.unwrap().tools;
    let mut seen: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    for (name, ro, destructive) in TABLE {
        let t = tools
            .iter()
            .find(|t| t.name == *name)
            .unwrap_or_else(|| panic!("tool {name} missing"));
        let a = t
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no annotations"));
        assert_eq!(a.read_only_hint, Some(*ro), "{name} readOnlyHint");
        if let Some(d) = destructive {
            assert_eq!(a.destructive_hint, Some(*d), "{name} destructiveHint");
        }
        assert!(
            t.description.as_deref().is_some_and(|d| d.len() > 20),
            "{name}: needs a description"
        );
        seen.retain(|n| n != name);
    }
    assert!(
        seen.is_empty(),
        "tools missing from the annotation table: {seen:?}"
    );
    let _ = c.cancel().await;
}

#[tokio::test]
async fn every_tool_forwards_to_the_addon_method_of_the_same_name() {
    let d = short_tempdir();
    let f = Fake::start(
        &d.path().join("f.sock"),
        Arc::new(|_, _| Reply::Ok(json!({"ok": true}))),
    );
    let c = connect_client(&f.path).await;
    let minimal = |name: &str| match name {
        "get_object" => json!({"name": "X"}),
        "create_primitive" | "create_object" => json!({"type": "cube"}),
        "set_transform" => json!({"names": ["X"], "location": [0, 0, 0]}),
        "delete_objects" => json!({"names": ["X"]}),
        "duplicate_object" => json!({"name": "X"}),
        "set_parent" => json!({"children": ["X"]}),
        "collection" => json!({"action": "create", "name": "C"}),
        "set_material" => json!({"object": "X"}),
        "add_modifier" => json!({"object": "X", "type": "SUBSURF"}),
        "apply_modifier" | "remove_modifier" => json!({"object": "X", "name": "M"}),
        "open_file" | "import_file" | "export_file" => json!({"path": "/tmp/x.obj"}),
        "run_python" => json!({"code": "pass"}),
        _ => json!({}),
    };
    for (name, _, _) in TABLE.iter().filter(|(n, _, _)| *n != "blender_guide") {
        let _ = call(&c, name, minimal(name)).await; // image tools report an error for the fake reply; irrelevant here
        assert_eq!(
            f.calls(name),
            1,
            "{name} must forward exactly one call to method '{name}'"
        );
    }
    let _ = c.cancel().await;
}

#[tokio::test]
async fn render_image_refuses_paths_outside_the_private_render_dir() {
    let d = short_tempdir();
    let evil = d.path().join("secret.png");
    std::fs::write(&evil, b"not an image").unwrap();
    let p = evil.to_str().unwrap().to_string();
    let f = Fake::start(
        &d.path().join("f.sock"),
        Arc::new(move |_, _| Reply::Ok(json!({"path": p}))),
    );
    let c = connect_client(&f.path).await;
    let o = call(&c, "render_image", json!({})).await;
    assert!(o.is_error);
    assert!(o.text.contains("refusing"), "{}", o.text);
    assert!(o.raw.content.iter().all(|b| b.as_image().is_none()));
    let _ = c.cancel().await;
}
