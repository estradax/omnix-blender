//! M2 end to end: every Phase-1 tool through MCP -> binary -> socket -> real headless Blender,
//! asserting the real Blender state afterwards (via an independent run_python channel).
mod support;
use serde_json::{Value, json};
use support::*;

async fn setup() -> Option<(Harness, Client)> {
    let bin = require_blender_opt()?;
    let h = Harness::start(&bin, true);
    let c = h.client().await;
    Some((h, c))
}

fn require_blender_opt() -> Option<std::path::PathBuf> {
    blender_or_skip()
}

async fn finish(mut h: Harness, c: Client) {
    let _ = c.cancel().await;
    h.shutdown().await;
}

fn names(v: &Value) -> Vec<String> {
    v["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn create_primitive_makes_real_geometry_for_every_type() {
    let Some((h, c)) = setup().await else { return };
    for (i, kind) in [
        "cube",
        "uv_sphere",
        "ico_sphere",
        "cylinder",
        "cone",
        "torus",
        "plane",
        "circle",
        "monkey",
    ]
    .iter()
    .enumerate()
    {
        let r = ok(
            &c,
            "create_primitive",
            json!({"type": kind, "name": format!("P_{kind}"), "location": [i as f64, 0.0, 1.0]}),
        )
        .await;
        assert_eq!(r["name"], format!("P_{kind}"));
        let info = ok(&c, "get_object", json!({"name": format!("P_{kind}")})).await;
        assert!(
            info["mesh"]["vertices"].as_u64().unwrap() >= 4,
            "{kind}: {info}"
        );
        assert_eq!(info["type"], "MESH");
    }
    // Rotation is degrees at the tool level, radians inside bpy; size is metres.
    ok(&c, "create_primitive", json!({"type": "cube", "name": "Box", "size": 4.0, "location": [1, 2, 3], "rotation": [0, 0, 90], "scale": [1, 2, 1]})).await;
    let st = h
        .inspect("o = bpy.data.objects['Box']\nresult = [list(o.location), o.rotation_euler.z, list(o.scale), list(o.dimensions), o.users_collection[0].name]")
        .await;
    assert_eq!(st[0], json!([1.0, 2.0, 3.0]));
    assert!((st[1].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-5);
    assert_eq!(st[3], json!([4.0, 8.0, 4.0]), "dimensions = size * scale");
    err(
        &c,
        "create_primitive",
        json!({"type": "dodecahedron"}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "create_primitive",
        json!({"type": "cube", "location": [1, 2]}),
        "deserialize", // wrong vector length: rejected by the schema before reaching Blender
    )
    .await;
    finish(h, c).await;
}

#[tokio::test]
async fn create_object_kinds() {
    let Some((h, c)) = setup().await else { return };
    ok(&c, "create_object", json!({"type": "light", "name": "Key", "light_type": "sun", "energy": 7.5, "color": [1, 0.5, 0.25], "location": [0, 0, 5]})).await;
    ok(&c, "create_object", json!({"type": "camera", "name": "Cam2", "focal_length": 85, "location": [0, -6, 2], "rotation": [90, 0, 0]})).await;
    ok(
        &c,
        "create_object",
        json!({"type": "empty", "name": "Pivot"}),
    )
    .await;
    ok(
        &c,
        "create_object",
        json!({"type": "text", "name": "Label", "text": "Hello"}),
    )
    .await;
    ok(
        &c,
        "create_object",
        json!({"type": "curve", "name": "Path"}),
    )
    .await;
    let st = h
        .inspect("L=bpy.data.objects['Key']; C=bpy.data.objects['Cam2']\nresult=[L.type, L.data.type, L.data.energy, list(L.data.color), C.type, C.data.lens, bpy.data.objects['Pivot'].type, bpy.data.objects['Label'].data.body, bpy.data.objects['Path'].type, len(bpy.data.objects['Path'].data.splines[0].bezier_points)]")
        .await;
    assert_eq!(st[0], "LIGHT");
    assert_eq!(st[1], "SUN");
    assert_eq!(st[2], 7.5);
    assert_eq!(st[3], json!([1.0, 0.5, 0.25]));
    assert_eq!(st[4], "CAMERA");
    assert!((st[5].as_f64().unwrap() - 85.0).abs() < 1e-4);
    assert_eq!(st[6], "EMPTY");
    assert_eq!(st[7], "Hello");
    assert_eq!(st[8], "CURVE");
    assert_eq!(st[9], 2);
    let l = ok(&c, "list_objects", json!({"type": "LIGHT"})).await;
    assert_eq!(l["total"], 2, "default Light + Key");
    err(
        &c,
        "create_object",
        json!({"type": "light", "light_type": "laser"}),
        "invalid_argument",
    )
    .await;
    finish(h, c).await;
}

#[tokio::test]
async fn set_transform_absolute_and_relative() {
    let Some((h, c)) = setup().await else { return };
    ok(&c, "create_primitive", json!({"type": "cube", "name": "A"})).await;
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "B", "location": [1, 1, 1]}),
    )
    .await;
    ok(&c, "set_transform", json!({"names": ["A", "B"], "location": [5, 0, 0], "rotation": [0, 0, 45], "scale": [2, 2, 2]})).await;
    ok(&c, "set_transform", json!({"names": ["A", "B"], "location": [1, 1, 1], "rotation": [0, 0, 45], "scale": [2, 1, 1], "relative": true})).await;
    let st = h.inspect("a=bpy.data.objects['A']; result=[list(a.location), a.rotation_euler.z, list(a.scale), list(bpy.data.objects['B'].location)]").await;
    assert_eq!(st[0], json!([6.0, 1.0, 1.0]));
    assert!((st[1].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-5);
    assert_eq!(st[2], json!([4.0, 2.0, 2.0]));
    assert_eq!(st[3], json!([6.0, 1.0, 1.0]));
    err(
        &c,
        "set_transform",
        json!({"names": ["A", "Nope"], "location": [0, 0, 0]}),
        "not_found",
    )
    .await;
    err(
        &c,
        "set_transform",
        json!({"names": ["A"]}),
        "invalid_argument",
    )
    .await;
    // The failed call must not have half-applied.
    let st = h
        .inspect("result=list(bpy.data.objects['A'].location)")
        .await;
    assert_eq!(st, json!([6.0, 1.0, 1.0]));
    finish(h, c).await;
}

#[tokio::test]
async fn delete_undo_redo_restore_real_state() {
    let Some((h, c)) = setup().await else { return };
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "Victim"}),
    )
    .await;
    assert!(names(&ok(&c, "list_objects", json!({})).await).contains(&"Victim".into()));
    err(
        &c,
        "delete_objects",
        json!({"names": ["Victim", "Ghost"]}),
        "not_found",
    )
    .await;
    assert_eq!(
        h.inspect("result='Victim' in bpy.data.objects").await,
        true,
        "nothing deleted on failure"
    );

    let d = ok(
        &c,
        "delete_objects",
        json!({"names": ["Victim"], "purge_orphans": true}),
    )
    .await;
    assert_eq!(d["deleted"], json!(["Victim"]));
    assert_eq!(
        h.inspect("result='Victim' in bpy.data.objects").await,
        false
    );
    assert_eq!(
        h.inspect("result='Victim' in bpy.data.meshes").await,
        false,
        "orphan mesh purged"
    );

    let u = ok(&c, "undo", json!({})).await;
    assert_eq!(u["steps_done"], 1);
    assert_eq!(
        h.inspect("result='Victim' in bpy.data.objects").await,
        true,
        "undo restored the object"
    );
    assert!(names(&ok(&c, "list_objects", json!({})).await).contains(&"Victim".into()));

    ok(&c, "redo", json!({})).await;
    assert_eq!(
        h.inspect("result='Victim' in bpy.data.objects").await,
        false,
        "redo deleted it again"
    );

    // One step per mutating call: undo twice goes back before the create.
    let u = ok(&c, "undo", json!({"steps": 2})).await;
    assert_eq!(u["steps_done"], 2);
    assert_eq!(
        h.inspect("result='Victim' in bpy.data.objects").await,
        false
    );
    assert_eq!(
        h.inspect("result=len(bpy.context.scene.objects)").await,
        3,
        "back to the factory scene"
    );
    finish(h, c).await;
}

#[tokio::test]
async fn organise_duplicate_parent_select_collection() {
    let Some((h, c)) = setup().await else { return };
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "Base", "location": [2, 0, 0]}),
    )
    .await;
    let d = ok(
        &c,
        "duplicate_object",
        json!({"name": "Base", "new_name": "Copy", "offset": [0, 3, 0]}),
    )
    .await;
    assert_eq!(d["name"], "Copy");
    let st = h.inspect("b=bpy.data.objects['Base']; c=bpy.data.objects['Copy']\nresult=[list(c.location), c.data is b.data, c.data.name]").await;
    assert_eq!(st[0], json!([2.0, 3.0, 0.0]));
    assert_eq!(st[1], false, "independent mesh data");

    ok(
        &c,
        "set_parent",
        json!({"children": ["Copy"], "parent": "Base"}),
    )
    .await;
    let info = ok(&c, "get_object", json!({"name": "Copy"})).await;
    assert_eq!(info["parent"], "Base");
    assert_eq!(
        info["world_location"],
        json!([2.0, 3.0, 0.0]),
        "keep_transform keeps the world position"
    );
    assert_eq!(
        ok(&c, "get_object", json!({"name": "Base"})).await["children"],
        json!(["Copy"])
    );
    err(
        &c,
        "set_parent",
        json!({"children": ["Base"], "parent": "Copy"}),
        "cycle",
    )
    .await;
    ok(&c, "set_parent", json!({"children": ["Copy"]})).await;
    assert_eq!(
        h.inspect("result=bpy.data.objects['Copy'].parent is None")
            .await,
        true
    );

    let s = ok(&c, "select_objects", json!({"names": ["Base", "Copy"]})).await;
    assert_eq!(s["selected"], json!(["Base", "Copy"]));
    assert_eq!(
        h.inspect(
            "result=sorted(o.name for o in bpy.context.view_layer.objects if o.select_get())"
        )
        .await,
        json!(["Base", "Copy"])
    );
    assert_eq!(
        h.inspect("result=bpy.context.view_layer.objects.active.name")
            .await,
        "Base"
    );
    ok(&c, "select_objects", json!({"mode": "clear"})).await;
    assert_eq!(
        h.inspect("result=[o.name for o in bpy.context.view_layer.objects if o.select_get()]")
            .await,
        json!([])
    );

    ok(
        &c,
        "collection",
        json!({"action": "create", "name": "Props"}),
    )
    .await;
    ok(
        &c,
        "collection",
        json!({"action": "move", "name": "Props", "objects": ["Base"]}),
    )
    .await;
    ok(
        &c,
        "collection",
        json!({"action": "link", "name": "Props", "objects": ["Copy"]}),
    )
    .await;
    let st = h.inspect("result=[sorted(c.name for c in bpy.data.objects['Base'].users_collection), sorted(c.name for c in bpy.data.objects['Copy'].users_collection)]").await;
    assert_eq!(st[0], json!(["Props"]));
    assert_eq!(st[1], json!(["Collection", "Props"]));
    let l = ok(&c, "list_objects", json!({"collection": "Props"})).await;
    assert_eq!(names(&l), ["Base", "Copy"]);
    err(
        &c,
        "collection",
        json!({"action": "create", "name": "Props"}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "collection",
        json!({"action": "move", "name": "Nope", "objects": ["Base"]}),
        "not_found",
    )
    .await;
    finish(h, c).await;
}

#[tokio::test]
async fn set_material_creates_updates_and_assigns() {
    let Some((h, c)) = setup().await else { return };
    ok(
        &c,
        "create_primitive",
        json!({"type": "uv_sphere", "name": "Ball"}),
    )
    .await;
    let r = ok(&c, "set_material", json!({"object": "Ball", "name": "Gold", "base_color": [1.0, 0.8, 0.2], "metallic": 1.0, "roughness": 0.25})).await;
    assert_eq!(r["created"], true);
    ok(&c, "set_material", json!({"object": "Ball", "name": "Gold", "alpha": 0.5, "emission": [1, 0, 0], "emission_strength": 3.0})).await;
    let st = h
        .inspect("m=bpy.data.materials['Gold']; n=m.node_tree.nodes['Principled BSDF']\nresult=[list(n.inputs['Base Color'].default_value), n.inputs['Metallic'].default_value, round(n.inputs['Roughness'].default_value,4), n.inputs['Alpha'].default_value, n.inputs['Emission Strength'].default_value, [s.material.name for s in bpy.data.objects['Ball'].material_slots], m.surface_render_method]")
        .await;
    assert_eq!(st[0], json!([1.0, 0.8, 0.2, 1.0]));
    assert_eq!(st[1], 1.0);
    assert_eq!(st[2], 0.25);
    assert_eq!(st[3], 0.5);
    assert_eq!(st[4], 3.0);
    assert_eq!(
        st[5],
        json!(["Gold"]),
        "same material is reused, not duplicated into a second slot"
    );
    assert_eq!(st[6], "BLENDED");
    assert_eq!(
        ok(&c, "get_object", json!({"name": "Ball"})).await["material_slots"],
        json!(["Gold"])
    );
    ok(&c, "create_object", json!({"type": "empty", "name": "E"})).await;
    err(
        &c,
        "set_material",
        json!({"object": "E", "base_color": [1, 0, 0]}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "set_material",
        json!({"object": "Ball", "metallic": 2.0}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "set_material",
        json!({"object": "Ball", "base_color": [1, 0]}),
        "invalid_argument",
    )
    .await;
    finish(h, c).await;
}

#[tokio::test]
async fn modifiers_add_apply_remove() {
    let Some((h, c)) = setup().await else { return };
    ok(&c, "create_primitive", json!({"type": "cube", "name": "M"})).await;
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "Cutter", "size": 1.0, "location": [1, 1, 1]}),
    )
    .await;
    let r = ok(&c, "add_modifier", json!({"object": "M", "type": "subsurf", "name": "Smooth", "properties": {"levels": 3, "render_levels": 3}})).await;
    assert_eq!(r["modifiers"], json!(["Smooth"]));
    let st = h.inspect("m=bpy.data.objects['M'].modifiers['Smooth']\nresult=[m.type, m.levels, m.render_levels]").await;
    assert_eq!(st, json!(["SUBSURF", 3, 3]));
    ok(&c, "add_modifier", json!({"object": "M", "type": "BOOLEAN", "name": "Cut", "properties": {"operation": "DIFFERENCE", "object": "Cutter"}})).await;
    assert_eq!(
        h.inspect("m=bpy.data.objects['M'].modifiers['Cut']\nresult=[m.operation, m.object.name]")
            .await,
        json!(["DIFFERENCE", "Cutter"])
    );
    assert_eq!(
        ok(&c, "get_object", json!({"name": "M"})).await["modifiers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // Validation: bad property / enum / type; nothing left behind.
    err(
        &c,
        "add_modifier",
        json!({"object": "M", "type": "SUBSURF", "name": "Bad", "properties": {"nope": 1}}),
        "invalid_argument",
    )
    .await;
    err(&c, "add_modifier", json!({"object": "M", "type": "BOOLEAN", "name": "Bad2", "properties": {"operation": "EXPLODE"}}), "invalid_argument").await;
    err(
        &c,
        "add_modifier",
        json!({"object": "M", "type": "SUBSURF", "name": "Bad3", "properties": {"levels": "high"}}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "add_modifier",
        json!({"object": "M", "type": "WOBBLE"}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "add_modifier",
        json!({"object": "Ghost", "type": "SUBSURF"}),
        "not_found",
    )
    .await;
    assert_eq!(
        h.inspect("result=[m.name for m in bpy.data.objects['M'].modifiers]")
            .await,
        json!(["Smooth", "Cut"])
    );

    ok(&c, "remove_modifier", json!({"object": "M", "name": "Cut"})).await;
    let before = h
        .inspect("result=len(bpy.data.objects['M'].data.vertices)")
        .await
        .as_u64()
        .unwrap();
    ok(
        &c,
        "apply_modifier",
        json!({"object": "M", "name": "Smooth"}),
    )
    .await;
    let st = h
        .inspect("o=bpy.data.objects['M']\nresult=[len(o.modifiers), len(o.data.vertices)]")
        .await;
    assert_eq!(st[0], 0);
    assert!(
        st[1].as_u64().unwrap() > before,
        "subdivision baked into the mesh"
    );
    err(
        &c,
        "apply_modifier",
        json!({"object": "M", "name": "Smooth"}),
        "not_found",
    )
    .await;
    finish(h, c).await;
}

#[tokio::test]
async fn render_settings_and_render_image_return_a_real_image() {
    let Some((h, c)) = setup().await else { return };
    let s = ok(&c, "set_render_settings", json!({"engine": "BLENDER_WORKBENCH", "resolution_x": 640, "resolution_y": 480, "resolution_percentage": 100, "output_format": "JPEG"})).await;
    assert_eq!(s["engine"], "BLENDER_WORKBENCH");
    assert_eq!(h.inspect("r=bpy.context.scene.render\nresult=[r.engine, r.resolution_x, r.resolution_y, r.image_settings.file_format]").await, json!(["BLENDER_WORKBENCH", 640, 480, "JPEG"]));
    err(
        &c,
        "set_render_settings",
        json!({"engine": "RAYTRACER"}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "set_render_settings",
        json!({"output_format": "BMPX"}),
        "invalid_argument",
    )
    .await;

    let o = call(&c, "render_image", json!({"max_edge": 100})).await;
    assert!(!o.is_error, "{}", o.text);
    let img = o
        .raw
        .content
        .iter()
        .find_map(|b| b.as_image())
        .expect("an MCP image block");
    assert_eq!(img.mime_type, "image/png");
    assert!(img.data.len() > 100);
    let meta = o.json();
    assert_eq!(meta["returned_width"], 100);
    assert_eq!(meta["returned_height"], 75);
    let path = meta["path"].as_str().unwrap();
    assert!(
        std::fs::metadata(path).unwrap().len() > 100,
        "full-size file exists on disk"
    );
    assert!(path.contains("omnix-render-"));
    let decoded = image::load_from_memory(&base64_decode(&img.data)).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (100, 75));
    // Render settings are restored after rendering.
    assert_eq!(h.inspect("r=bpy.context.scene.render\nresult=[r.image_settings.file_format, r.filepath != '' and r.filepath.startswith('/tmp/omnix-render')]").await[0], "JPEG");

    err(&c, "render_image", json!({"camera": "Ghost"}), "not_found").await;
    h.inspect("bpy.context.scene.camera=None").await;
    err(&c, "render_image", json!({}), "no camera").await;
    finish(h, c).await;
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

#[tokio::test]
async fn viewport_screenshot_is_unsupported_headless() {
    let Some((h, c)) = setup().await else { return };
    let t = err(&c, "viewport_screenshot", json!({}), "unsupported").await;
    assert!(t.contains("GUI"), "{t}");
    finish(h, c).await;
}

#[tokio::test]
async fn file_tools_save_open_new_with_unsaved_changes_guard() {
    let Some((h, c)) = setup().await else { return };
    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "Saved"}),
    )
    .await;
    let path = h.dir.path().canonicalize().unwrap().join("scene.blend");
    let r = ok(&c, "save_file", json!({"path": path.to_str().unwrap()})).await;
    assert_eq!(r["file"], path.to_str().unwrap());
    assert!(path.metadata().unwrap().len() > 1000);
    let info = ok(&c, "scene_info", json!({})).await;
    assert_eq!(info["file"], path.to_str().unwrap());

    ok(
        &c,
        "create_primitive",
        json!({"type": "cube", "name": "Unsaved"}),
    )
    .await;
    assert_eq!(ok(&c, "scene_info", json!({})).await["unsaved"], true);
    err(
        &c,
        "open_file",
        json!({"path": path.to_str().unwrap()}),
        "discard_changes",
    )
    .await;
    err(&c, "new_file", json!({}), "discard_changes").await;
    assert_eq!(
        h.inspect("result='Unsaved' in bpy.data.objects").await,
        true,
        "refusal changed nothing"
    );

    ok(
        &c,
        "open_file",
        json!({"path": path.to_str().unwrap(), "discard_changes": true}),
    )
    .await;
    assert_eq!(
        h.inspect("result=['Saved' in bpy.data.objects, 'Unsaved' in bpy.data.objects]")
            .await,
        json!([true, false])
    );
    err(
        &c,
        "open_file",
        json!({"path": h.dir.path().join("nodir/here.blend").to_str().unwrap(), "discard_changes": true}),
        "not_found",
    )
    .await;

    ok(&c, "save_file", json!({})).await; // in place
    assert_eq!(
        ok(&c, "scene_info", json!({})).await["unsaved"],
        false,
        "saving clears the unsaved flag"
    );
    let n = ok(&c, "new_file", json!({"empty": true})).await;
    assert_eq!(n["objects"], 0);
    assert_eq!(h.inspect("result=len(bpy.data.objects)").await, 0);
    err(&c, "save_file", json!({}), "never been saved").await;
    finish(h, c).await;
}

#[tokio::test]
async fn import_export_round_trip_all_formats() {
    let Some((h, c)) = setup().await else { return };
    ok(
        &c,
        "create_primitive",
        json!({"type": "monkey", "name": "Suzanne"}),
    )
    .await;
    let verts = h
        .inspect("result=len(bpy.data.objects['Suzanne'].data.vertices)")
        .await
        .as_u64()
        .unwrap();
    for ext in ["obj", "stl", "ply", "usdc", "abc", "fbx", "glb"] {
        let p = h.dir.path().join(format!("out.{ext}"));
        let e = ok(
            &c,
            "export_file",
            json!({"path": p.to_str().unwrap(), "selected_only": false}),
        )
        .await;
        assert!(e["bytes"].as_u64().unwrap() > 50, "{ext}: {e}");
        assert!(p.exists(), "{ext} not written");
        ok(
            &c,
            "new_file",
            json!({"discard_changes": true, "empty": true}),
        )
        .await;
        let i = ok(&c, "import_file", json!({"path": p.to_str().unwrap()})).await;
        assert!(i["count"].as_u64().unwrap() >= 1, "{ext}: {i}");
        let total: u64 = h
            .inspect("result=sum(len(o.data.vertices) for o in bpy.data.objects if o.type=='MESH')")
            .await
            .as_u64()
            .unwrap();
        assert!(total > 0, "{ext}: imported object has geometry");
        if ext == "obj" || ext == "ply" || ext == "stl" {
            assert!(total >= verts / 2, "{ext}: {total} vs {verts}");
        }
        // Put the monkey back for the next format.
        ok(
            &c,
            "create_primitive",
            json!({"type": "monkey", "name": "Suzanne"}),
        )
        .await;
    }
    err(
        &c,
        "import_file",
        json!({"path": h.dir.path().join("nope.obj").to_str().unwrap()}),
        "not_found",
    )
    .await;
    err(
        &c,
        "import_file",
        json!({"path": h.dir.path().join("out.obj").to_str().unwrap(), "format": "dwg"}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "export_file",
        json!({"path": h.dir.path().join("x.unknown").to_str().unwrap()}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "export_file",
        json!({"path": h.dir.path().join("nodir/x.obj").to_str().unwrap()}),
        "not_found",
    )
    .await;
    finish(h, c).await;
}

#[tokio::test]
async fn run_python_returns_result_stdout_and_errors() {
    let Some((h, c)) = setup().await else { return };
    let r = ok(
        &c,
        "run_python",
        json!({"code": "print('hi')\nresult = len(bpy.data.objects)"}),
    )
    .await;
    assert_eq!(r["stdout"], "hi\n");
    assert_eq!(r["result"], 3);
    let o = call(&c, "run_python", json!({"code": "1/0"})).await;
    assert!(o.is_error);
    assert_eq!(o.json()["error"]["type"], "ZeroDivisionError");
    err(&c, "run_python", json!({"code": "  "}), "invalid_argument").await;
    // The add-on survives a script that raises SystemExit.
    let o = call(&c, "run_python", json!({"code": "raise SystemExit(3)"})).await;
    assert!(o.is_error);
    ok(&c, "scene_info", json!({})).await;
    finish(h, c).await;
}

#[tokio::test]
async fn invalid_arguments_and_unknown_objects_have_stable_codes() {
    let Some((h, c)) = setup().await else { return };
    err(&c, "get_object", json!({"name": "Nope"}), "not_found").await;
    err(
        &c,
        "list_objects",
        json!({"limit": 5000}),
        "invalid_argument",
    )
    .await;
    err(
        &c,
        "delete_objects",
        json!({"names": []}),
        "invalid_argument",
    )
    .await;
    err(&c, "undo", json!({"steps": 1000}), "invalid_argument").await;
    finish(h, c).await;
}
