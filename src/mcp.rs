//! The MCP server: tool schemas, annotations, and translation to add-on calls.

use crate::error::LinkError;
use crate::link::{DEFAULT_TIMEOUT, Link};
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const GUIDE: &str = "\
Blender tools: how to use them well.

- Units are metres, the world is Z-up. Rotations are given in DEGREES by these tools (Blender's own Python API uses radians).
- Names are case-sensitive and unique per object. Blender renames on collision (Cube -> Cube.001): use the name returned by the create tools.
- Start with scene_info, then list_objects (paged with limit/offset) and get_object. Results are compact and never contain raw meshes.
- Every mutating tool is one undo step; call undo to revert it.
- Prefer the structured tools. run_python exists but is disabled unless the user enables 'Allow run_python' in the add-on preferences.
- Long operations (render_image, import/export of big files) block Blender's UI and its request queue; other requests answer 'busy' until they finish.
- viewport_screenshot only works in the Blender GUI, not headless.
- Errors carry a code: not_found, invalid_argument, unsupported, disabled, blender_error, timeout, busy, not_connected.
- If Blender is not reachable, ask the user to open Blender and press Start in the sidebar (N panel > Omnix).";

#[derive(Clone)]
#[allow(dead_code)]
pub struct OmnixBlender {
    link: Arc<Link>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListObjectsArgs {
    /// Only objects of this type, e.g. MESH, CAMERA, LIGHT, EMPTY, CURVE, FONT.
    #[serde(rename = "type")]
    pub object_type: Option<String>,
    /// Only objects in this collection (including nested ones).
    pub collection: Option<String>,
    /// Case-sensitive glob on the object name, e.g. "Wall*".
    pub name_glob: Option<String>,
    /// Page size (1-1000, default 100).
    pub limit: Option<u32>,
    /// Number of objects to skip (default 0).
    pub offset: Option<u32>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunPythonArgs {
    /// Python source to execute inside Blender (bpy, bmesh, mathutils are in scope). Assign to `result` to return a JSON-able value.
    pub code: String,
    /// Seconds to wait before giving up (default 30). A running script cannot be interrupted.
    pub timeout_s: Option<f64>,
    /// Set false to skip creating an undo step for this call.
    pub undo: Option<bool>,
}

type Vec3 = [f64; 3];

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameArg {
    /// Object name (case-sensitive).
    pub name: String,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePrimitiveArgs {
    /// cube, uv_sphere, ico_sphere, cylinder, cone, torus, plane, circle or monkey.
    #[serde(rename = "type")]
    pub kind: String,
    /// Overall size in metres: edge length for cube/plane, diameter/height for round shapes (default 2).
    pub size: Option<f64>,
    /// Segments for round shapes (3-256, default 32).
    pub segments: Option<u32>,
    pub name: Option<String>,
    /// World location in metres.
    pub location: Option<Vec3>,
    /// Euler rotation in degrees.
    pub rotation: Option<Vec3>,
    pub scale: Option<Vec3>,
    /// Collection to put it in (default: the active collection).
    pub collection: Option<String>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateObjectArgs {
    /// empty, camera, light, text or curve.
    #[serde(rename = "type")]
    pub kind: String,
    pub name: Option<String>,
    pub location: Option<Vec3>,
    /// Euler rotation in degrees.
    pub rotation: Option<Vec3>,
    pub scale: Option<Vec3>,
    pub collection: Option<String>,
    /// Lights: point, sun, spot or area (default point).
    pub light_type: Option<String>,
    /// Lights: power (W for point/spot/area, strength for sun).
    pub energy: Option<f64>,
    /// Lights: RGB 0-1.
    pub color: Option<[f64; 3]>,
    /// Cameras: focal length in mm (default 50).
    pub focal_length: Option<f64>,
    /// Text objects: the text.
    pub text: Option<String>,
    /// Empties: display size.
    pub size: Option<f64>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTransformArgs {
    /// Objects to change.
    pub names: Vec<String>,
    pub location: Option<Vec3>,
    /// Euler rotation in degrees.
    pub rotation: Option<Vec3>,
    pub scale: Option<Vec3>,
    /// Add to location/rotation and multiply scale instead of replacing (default false).
    pub relative: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteObjectsArgs {
    pub names: Vec<String>,
    /// Also purge datablocks (meshes, materials...) left unused (default false).
    pub purge_orphans: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateArgs {
    pub name: String,
    pub new_name: Option<String>,
    /// Share mesh data with the original (default false: independent copy).
    pub linked: Option<bool>,
    /// Offset added to the copy's location.
    pub offset: Option<Vec3>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParentArgs {
    pub children: Vec<String>,
    /// Parent object name; omit or null to clear the parent.
    pub parent: Option<String>,
    /// Keep the world transform (default true).
    pub keep_transform: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectArgs {
    /// Objects to select (not needed for mode=clear).
    pub names: Option<Vec<String>>,
    /// replace (default), add, remove or clear.
    pub mode: Option<String>,
    /// Make the first object active (default true).
    pub make_active: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectionArgs {
    /// create, move (unlink from other collections) or link.
    pub action: String,
    /// Collection name.
    pub name: String,
    /// create: parent collection (default: the scene collection).
    pub parent: Option<String>,
    /// move/link: objects to put in the collection.
    pub objects: Option<Vec<String>>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMaterialArgs {
    /// Object to assign the material to.
    pub object: String,
    /// Material name; created if missing, updated if it exists (default "<object>_Mat").
    pub name: Option<String>,
    /// RGB or RGBA, 0-1.
    pub base_color: Option<Vec<f64>>,
    pub metallic: Option<f64>,
    pub roughness: Option<f64>,
    /// 0-1; below 1 switches to blended transparency.
    pub alpha: Option<f64>,
    /// Emission colour RGB(A) 0-1.
    pub emission: Option<Vec<f64>>,
    pub emission_strength: Option<f64>,
    /// Material slot index (default: existing slot of this material, else the first slot, else a new one).
    pub slot: Option<u32>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddModifierArgs {
    pub object: String,
    /// Modifier type, e.g. SUBSURF, BEVEL, MIRROR, ARRAY, BOOLEAN, SOLIDIFY, DECIMATE, TRIANGULATE.
    #[serde(rename = "type")]
    pub kind: String,
    pub name: Option<String>,
    /// Property map using Blender's property names, e.g. {"levels": 2} for SUBSURF, {"operation": "DIFFERENCE", "object": "Cutter"} for BOOLEAN.
    pub properties: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModifierRefArgs {
    pub object: String,
    /// Modifier name (see get_object).
    pub name: String,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderSettingsArgs {
    /// BLENDER_EEVEE, CYCLES or BLENDER_WORKBENCH.
    pub engine: Option<String>,
    pub resolution_x: Option<u32>,
    pub resolution_y: Option<u32>,
    pub resolution_percentage: Option<u32>,
    pub samples: Option<u32>,
    /// PNG, JPEG, OPEN_EXR, ...
    pub output_format: Option<String>,
    pub output_path: Option<String>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderImageArgs {
    /// Camera object to render from (default: the scene camera).
    pub camera: Option<String>,
    pub samples: Option<u32>,
    /// Temporarily override resolution percentage (e.g. 25 for a quick preview).
    pub resolution_percentage: Option<u32>,
    /// Longest edge of the returned image in pixels (default 1280). The full-size file path is returned too.
    #[serde(skip_serializing)]
    pub max_edge: Option<u32>,
    /// Seconds to wait (default 600).
    #[serde(skip_serializing)]
    pub timeout_s: Option<f64>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotArgs {
    /// Longest edge of the returned image in pixels (default 1280).
    #[serde(skip_serializing)]
    pub max_edge: Option<u32>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveFileArgs {
    /// Where to save (.blend added if missing). Omit to save to the current file. Must be inside the .blend's folder or an allowed folder.
    pub path: Option<String>,
    /// Replace an existing file at `path` (default false: refused).
    pub overwrite: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenFileArgs {
    pub path: String,
    /// Required to open when the current file has unsaved changes (they are lost).
    pub discard_changes: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewFileArgs {
    /// Required when the current file has unsaved changes (they are lost).
    pub discard_changes: Option<bool>,
    /// Start with an empty scene instead of the default cube/camera/light.
    pub empty: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportFileArgs {
    pub path: String,
    /// obj, stl, ply, usd, abc, fbx or gltf (glb). Inferred from the extension when omitted.
    pub format: Option<String>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportFileArgs {
    /// Destination; the directory must exist and be inside the .blend's folder or an allowed folder. The extension must match the format.
    pub path: String,
    /// obj, stl, ply, usd, abc, fbx or gltf (glb). Inferred from the extension when omitted; must agree with it.
    pub format: Option<String>,
    /// Replace an existing file (and side files such as .mtl); default false: refused.
    pub overwrite: Option<bool>,
    /// Export only selected objects (default false).
    pub selected_only: Option<bool>,
}

#[derive(Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepsArgs {
    /// Number of undo/redo steps (1-100, default 1).
    pub steps: Option<u32>,
}

impl OmnixBlender {
    pub fn new(link: Arc<Link>) -> Self {
        Self {
            link,
            tool_router: Self::tool_router(),
        }
    }

    /// Call the add-on and turn the outcome into an MCP tool result (errors are tool errors, never hangs).
    pub async fn invoke(
        &self,
        method: &str,
        params: Value,
        read_only: bool,
        timeout: Duration,
    ) -> CallToolResult {
        match self.link.call(method, params, timeout, read_only).await {
            Ok(v) => CallToolResult::success(vec![ContentBlock::text(compact(&v))]),
            Err(e) => tool_error(&e),
        }
    }
}

pub fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

pub fn tool_error(e: &LinkError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "error [{}]: {}",
        e.code(),
        e
    ))])
}

/// Drop `None` fields so the add-on sees only what the caller supplied.
pub fn params_of<T: serde::Serialize>(args: &T) -> Value {
    let mut v = serde_json::to_value(args).unwrap_or_else(|_| json!({}));
    if let Value::Object(m) = &mut v {
        m.retain(|_, x| !x.is_null());
    }
    v
}

#[tool_router]
impl OmnixBlender {
    #[tool(
        description = "Short guide to using the Blender tools well (units, naming, undo, limits). Read this first.",
        annotations(read_only_hint = true)
    )]
    async fn blender_guide(&self) -> String {
        GUIDE.to_string()
    }

    #[tool(
        description = "Run arbitrary Python inside Blender and return {result, stdout, error?}. DISABLED by default: the user must tick 'Allow run_python' in the add-on preferences. Prefer the structured tools.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn run_python(&self, Parameters(a): Parameters<RunPythonArgs>) -> CallToolResult {
        self.run_python_impl(a).await
    }

    #[tool(
        description = "Summary of the open Blender scene: file, unsaved state, units, frame range, render engine, object/material/camera/light counts, active object, selection.",
        annotations(read_only_hint = true)
    )]
    async fn scene_info(&self) -> CallToolResult {
        self.invoke("scene_info", json!({}), true, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "List objects (name, type, visibility, parent, location), sorted by name, paged with limit/offset. Reports total and truncated.",
        annotations(read_only_hint = true)
    )]
    async fn list_objects(&self, Parameters(a): Parameters<ListObjectsArgs>) -> CallToolResult {
        self.invoke("list_objects", params_of(&a), true, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Full detail of one object: transform (location, rotation in degrees, scale), world location, dimensions, parent/children, collections, modifiers, material slots, mesh statistics and world bounding box.",
        annotations(read_only_hint = true)
    )]
    async fn get_object(&self, Parameters(a): Parameters<NameArg>) -> CallToolResult {
        self.invoke("get_object", params_of(&a), true, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Create a mesh primitive (cube, uv_sphere, ico_sphere, cylinder, cone, torus, plane, circle, monkey). Returns the final object name (Blender may add .001).",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_primitive(
        &self,
        Parameters(a): Parameters<CreatePrimitiveArgs>,
    ) -> CallToolResult {
        self.invoke("create_primitive", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Create an empty, camera, light (point/sun/spot/area with energy and colour), text or curve object.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_object(&self, Parameters(a): Parameters<CreateObjectArgs>) -> CallToolResult {
        self.invoke("create_object", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Set or (with relative=true) offset location / rotation (degrees) / scale of one or more objects.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn set_transform(&self, Parameters(a): Parameters<SetTransformArgs>) -> CallToolResult {
        self.invoke("set_transform", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Delete objects by name (all names must exist, otherwise nothing is deleted). Optionally purge orphaned data. Undoable with undo.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn delete_objects(&self, Parameters(a): Parameters<DeleteObjectsArgs>) -> CallToolResult {
        self.invoke("delete_objects", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Duplicate an object (independent data unless linked=true), optionally renaming and offsetting it.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn duplicate_object(&self, Parameters(a): Parameters<DuplicateArgs>) -> CallToolResult {
        self.invoke("duplicate_object", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Parent objects to another object (or clear the parent), keeping their world transform by default.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn set_parent(&self, Parameters(a): Parameters<SetParentArgs>) -> CallToolResult {
        self.invoke("set_parent", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Change the selection (replace/add/remove/clear) and make the first object active.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn select_objects(&self, Parameters(a): Parameters<SelectArgs>) -> CallToolResult {
        self.invoke("select_objects", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Collections: create a collection, or move/link objects into one.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn collection(&self, Parameters(a): Parameters<CollectionArgs>) -> CallToolResult {
        self.invoke("collection", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Create or update a Principled BSDF material (base colour, metallic, roughness, alpha, emission) and assign it to an object slot.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn set_material(&self, Parameters(a): Parameters<SetMaterialArgs>) -> CallToolResult {
        self.invoke("set_material", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Add a modifier of any Blender type with a validated property map. Bad properties are rejected and the modifier is not left behind.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn add_modifier(&self, Parameters(a): Parameters<AddModifierArgs>) -> CallToolResult {
        self.invoke("add_modifier", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Apply a modifier to the object's mesh (bakes it into the geometry).",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn apply_modifier(&self, Parameters(a): Parameters<ModifierRefArgs>) -> CallToolResult {
        self.invoke("apply_modifier", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Remove a modifier from an object without applying it.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn remove_modifier(&self, Parameters(a): Parameters<ModifierRefArgs>) -> CallToolResult {
        self.invoke("remove_modifier", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Set render engine (BLENDER_EEVEE, CYCLES, BLENDER_WORKBENCH), resolution, samples and output format/path. Returns the resulting settings.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn set_render_settings(
        &self,
        Parameters(a): Parameters<RenderSettingsArgs>,
    ) -> CallToolResult {
        self.invoke("set_render_settings", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Render the current frame and return it as an image (downscaled to max_edge) plus the full-size file path. BLOCKS Blender's UI until done; other requests answer 'busy'.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn render_image(&self, Parameters(a): Parameters<RenderImageArgs>) -> CallToolResult {
        let timeout = Duration::from_secs_f64(a.timeout_s.unwrap_or(600.0).clamp(1.0, 3600.0));
        let max_edge = a.max_edge.unwrap_or(DEFAULT_MAX_EDGE);
        let r = self
            .link
            .call("render_image", params_of(&a), timeout, false)
            .await;
        image_result(r, max_edge).await
    }

    #[tool(
        description = "Screenshot of the active 3D viewport. GUI only: fails with 'unsupported' when Blender runs headless.",
        annotations(read_only_hint = true)
    )]
    async fn viewport_screenshot(
        &self,
        Parameters(a): Parameters<ScreenshotArgs>,
    ) -> CallToolResult {
        let r = self
            .link
            .call("viewport_screenshot", json!({}), DEFAULT_TIMEOUT, true)
            .await;
        image_result(r, a.max_edge.unwrap_or(DEFAULT_MAX_EDGE)).await
    }

    #[tool(
        description = "Save the scene (to `path`, or in place if it already has a file). Refuses to replace an existing file at `path` unless overwrite=true. Paths must be in the .blend's folder or an allowed folder.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn save_file(&self, Parameters(a): Parameters<SaveFileArgs>) -> CallToolResult {
        self.invoke("save_file", params_of(&a), false, Duration::from_secs(120))
            .await
    }

    #[tool(
        description = "Open a .blend file. Refuses when the current file has unsaved changes unless discard_changes=true.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn open_file(&self, Parameters(a): Parameters<OpenFileArgs>) -> CallToolResult {
        self.invoke("open_file", params_of(&a), false, Duration::from_secs(120))
            .await
    }

    #[tool(
        description = "Start a new scene. Refuses when the current file has unsaved changes unless discard_changes=true.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn new_file(&self, Parameters(a): Parameters<NewFileArgs>) -> CallToolResult {
        self.invoke("new_file", params_of(&a), false, Duration::from_secs(60))
            .await
    }

    #[tool(
        description = "Import obj, stl, ply, usd/usdc/usda/usdz, abc, fbx or gltf/glb. Returns the new object names.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn import_file(&self, Parameters(a): Parameters<ImportFileArgs>) -> CallToolResult {
        self.invoke(
            "import_file",
            params_of(&a),
            false,
            Duration::from_secs(300),
        )
        .await
    }

    #[tool(
        description = "Export the scene (or the selection) to obj, stl, ply, usd, abc, fbx or gltf/glb. Refuses to replace existing files unless overwrite=true; reports side files (e.g. .mtl). Paths must be in the .blend's folder or an allowed folder.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn export_file(&self, Parameters(a): Parameters<ExportFileArgs>) -> CallToolResult {
        self.invoke(
            "export_file",
            params_of(&a),
            false,
            Duration::from_secs(300),
        )
        .await
    }

    #[tool(
        description = "Undo the last N changes (each mutating tool call is one step).",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn undo(&self, Parameters(a): Parameters<StepsArgs>) -> CallToolResult {
        self.invoke("undo", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }

    #[tool(
        description = "Redo the last N undone changes.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn redo(&self, Parameters(a): Parameters<StepsArgs>) -> CallToolResult {
        self.invoke("redo", params_of(&a), false, DEFAULT_TIMEOUT)
            .await
    }
}

impl OmnixBlender {
    async fn run_python_impl(&self, a: RunPythonArgs) -> CallToolResult {
        let timeout = Duration::from_secs_f64(a.timeout_s.unwrap_or(30.0).clamp(1.0, 3600.0));
        let params = params_of(&a);
        match self.link.call("run_python", params, timeout, false).await {
            Ok(v) => {
                let failed = v.get("error").is_some_and(|e| !e.is_null());
                let b = ContentBlock::text(compact(&v));
                if failed {
                    CallToolResult::error(vec![b])
                } else {
                    CallToolResult::success(vec![b])
                }
            }
            Err(e) => tool_error(&e),
        }
    }
}

pub const DEFAULT_MAX_EDGE: u32 = 1280;
const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;

/// Turn an add-on image result `{path, ...}` into an MCP image block (downscaled) plus the JSON metadata.
async fn image_result(r: Result<Value, LinkError>, max_edge: u32) -> CallToolResult {
    let meta = match r {
        Ok(v) => v,
        Err(e) => return tool_error(&e),
    };
    let Some(path) = meta
        .get("path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
    else {
        return tool_error(&LinkError::Protocol("add-on returned no image path".into()));
    };
    let max_edge = max_edge.clamp(16, 8192);
    match tokio::task::spawn_blocking(move || load_scaled_png(&path, max_edge)).await {
        Ok(Ok((b64, w, h))) => {
            let mut meta = meta;
            if let Value::Object(m) = &mut meta {
                m.insert("returned_width".into(), json!(w));
                m.insert("returned_height".into(), json!(h));
            }
            CallToolResult::success(vec![
                ContentBlock::image(b64, "image/png"),
                ContentBlock::text(compact(&meta)),
            ])
        }
        Ok(Err(msg)) => tool_error(&LinkError::Io(msg)),
        Err(e) => tool_error(&LinkError::Io(e.to_string())),
    }
}

/// Read an image the add-on wrote, accepting only what the add-on itself creates: a regular file, owned by us,
/// directly inside a private (`0700`, owned by us, not a symlink) `omnix-render-*` directory that lies under the
/// system temp dir. The file is opened once with `O_NOFOLLOW` and checked through the open descriptor, so a swap
/// between check and use cannot redirect the read. (The add-on and this server must share `TMPDIR`.)
pub fn load_scaled_png(
    path: &std::path::Path,
    max_edge: u32,
) -> Result<(String, u32, u32), String> {
    use base64::Engine as _;
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let uid = unsafe { libc::getuid() };
    let refuse = |why: &str| Err(format!("refusing to read the image: {why}"));
    let canon = std::fs::canonicalize(path).map_err(|e| format!("image missing: {e}"))?;
    let temp =
        std::fs::canonicalize(std::env::temp_dir()).map_err(|e| format!("no temp dir: {e}"))?;
    let Some(dir) = canon.parent() else {
        return refuse("no parent directory");
    };
    if !dir.starts_with(&temp) {
        return refuse("it is not under the system temp directory");
    }
    if !dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("omnix-render-"))
    {
        return refuse("it is not in an omnix-render-* directory");
    }
    let dmeta = std::fs::symlink_metadata(dir).map_err(|e| format!("directory missing: {e}"))?;
    if !dmeta.is_dir() || dmeta.uid() != uid || dmeta.permissions().mode() & 0o777 != 0o700 {
        return refuse("the render directory is not a private (0700) directory owned by you");
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&canon)
        .map_err(|e| format!("cannot open image: {e}"))?;
    let fmeta = file
        .metadata()
        .map_err(|e| format!("cannot stat image: {e}"))?;
    if !fmeta.is_file() || fmeta.uid() != uid {
        return refuse("it is not a regular file owned by you");
    }
    if fmeta.len() > MAX_IMAGE_BYTES {
        return refuse("it is too large");
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read image: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("cannot decode image: {e}"))?;
    let img = if img.width().max(img.height()) > max_edge {
        img.resize(max_edge, max_edge, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let (w, h) = (img.width(), img.height());
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("cannot encode PNG: {e}"))?;
    Ok((
        base64::engine::general_purpose::STANDARD.encode(out.into_inner()),
        w,
        h,
    ))
}

#[tool_handler]
impl ServerHandler for OmnixBlender {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(GUIDE)
    }
}
