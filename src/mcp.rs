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
pub struct RunPythonArgs {
    /// Python source to execute inside Blender (bpy, bmesh, mathutils are in scope). Assign to `result` to return a JSON-able value.
    pub code: String,
    /// Seconds to wait before giving up (default 30). A running script cannot be interrupted.
    pub timeout_s: Option<f64>,
    /// Set false to skip creating an undo step for this call.
    pub undo: Option<bool>,
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

#[tool_handler]
impl ServerHandler for OmnixBlender {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(GUIDE)
    }
}
