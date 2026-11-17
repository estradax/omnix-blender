# omnix-blender — plan

A Blender MCP server written in Rust. It replaces the third-party Python `blender-mcp`, which the owner
considers poor. It is a standalone stdio MCP server that works with any MCP client and is designed to sit behind
[omnix](https://github.com/estradax/omnix) (the code-mode gateway: `~/project/omnix`) without special support.

Status: **planning only. No code exists yet.** This file is the handover brief. Decisions marked **[DECISION]**
were chosen by the planner because the owner skipped the questions; they are defaults, not sacred. If you
disagree with one for a good technical reason, change it and record why in `docs/DECISIONS.md`.

## 1. Constraint that shapes everything

Blender's API (`bpy`) exists only inside Blender's embedded Python, which is not thread-safe and must be driven from
Blender's main thread. A Rust program cannot call it. So the system has two parts:

```
MCP client ──stdio──▶ omnix-blender (Rust, rmcp)  ──local socket, NDJSON──▶  Blender add-on (Python)
                                                                              └─ main-thread dispatcher ─▶ bpy
```

- **Rust server** (`omnix-blender`): speaks MCP, validates arguments, owns the tool schemas, talks to the add-on.
- **Add-on** (`addon/`, pure Python, no third-party deps): runs a socket server on a background thread, hands each
  request to Blender's **main thread**, executes it with `bpy`, returns JSON.

Installed Blender on the owner's machine: **`/Applications/Blender.app`, Blender 5.3.0** (build 2026-11-10). User
config dirs exist for 5.1 and 5.3 under `~/Library/Application Support/Blender/`. Target Blender **4.2 LTS and newer**
(extensions era) but develop and test against 5.3. macOS (Apple Silicon) first; Linux should work; Windows later.

## 2. Decisions

| | |
|---|---|
| **[DECISION] Link** | Live GUI session through an add-on over a **Unix domain socket** (not TCP). A headless mode comes later using the *same* dispatcher (see §5), so the add-on code is reused. |
| **[DECISION] Tool surface** | A moderate set of **structured, typed tools** for common work **plus** `run_python` as an escape hatch, which is **disabled by default** and enabled by an add-on preference. |
| **[DECISION] Location** | `~/project/omnix-blender`, own git repo, **local only / private**. Do not create a GitHub remote or publish without the owner's explicit approval. |
| Names | Crate and binary: `omnix-blender`. Add-on id: `omnix_blender`. |
| License | `MIT OR Apache-2.0`. Copy `LICENSE-MIT` / `LICENSE-APACHE` from `~/project/omnix` (same holder). |
| MCP SDK | `rmcp` **3.10** server with the `#[tool_router]` pattern. The working reference is `~/project/omnix/examples/mock_mcp.rs`. Reuse its structure. |

### Why a Unix socket and not TCP (security)

`run_python` is remote code execution by design. A loopback **TCP** port is reachable by *any* local process and by web
pages through DNS-rebinding and CSRF-style tricks, which is the well-known weakness of existing Blender MCP add-ons.
A Unix socket in a `0700` directory with `0600` permissions is reachable only by the same user. Requirements:

1. Socket path: `$OMNIX_BLENDER_SOCKET`, else `<config_dir>/omnix-blender/blender.sock` where config_dir is the platform
   config dir (macOS `~/Library/Application Support`). Directory mode `0700`, socket mode `0600`; refuse to start if
   the directory is group/world writable or not owned by the user.
2. The add-on **never** binds a TCP port by default. A TCP+token fallback for Windows is a later, explicit opt-in.
3. `run_python` is off unless the user ticks it in the add-on preferences, and the Rust tool says exactly how to
   enable it when called while disabled. It is also marked destructive/non-read-only so an omnix policy can deny it
   (`"deny": ["blender.run_python"]`).
4. Never execute anything received before the connection is established and the protocol version is checked.

## 3. Protocol (Rust ⇄ add-on)

Newline-delimited JSON over the socket, one request per line, one response per line, matched by `id`.

```jsonc
// request
{"v":1,"id":7,"method":"objects.create_primitive","params":{"type":"cube","name":"Box","location":[0,0,1]}}
// response (success)
{"v":1,"id":7,"ok":true,"result":{"name":"Box","type":"MESH","location":[0,0,1]}}
// response (error)
{"v":1,"id":7,"ok":false,"error":{"code":"not_found","message":"object 'Foo' does not exist"}}
```

- First message after connect is `hello` → `{"v":1,"blender":"5.3.0","addon":"0.1.0","features":[…]}`. The Rust side
  refuses an incompatible `v` with a clear error telling the user to update the add-on.
- Error `code` is a stable enum: `not_found`, `invalid_argument`, `unsupported`, `disabled` (e.g. run_python off),
  `blender_error` (a Python exception, with a short traceback), `timeout`, `busy`.
- Limits: max line 16 MiB; per-request timeout (default 30 s, renders 600 s); the Rust side never sends a new
  request while one is in flight unless the add-on advertised concurrency (it will not in v1; serialize requests).
- Large binary results (renders, screenshots) are written to a **file** by the add-on in a private temp dir and the
  response carries the path and size. The Rust server reads it and returns an MCP image content block (downscaled to
  a configurable max edge, default 1280 px) plus the path. Never inline megabytes of base64 through the socket.

## 4. Add-on design (Python, runs inside Blender)

Packaging: the **Blender extension** format (`blender_manifest.toml`, `schema_version = "1.0.0"`, `type = "add-on"`,
`blender_version_min = "4.2.0"`, license `SPDX:MIT`, no network permission needed for a local socket; verify the exact
permission keys). **Verify the manifest keys and the install procedure against Blender 5.3 by actually installing it**
(`blender --command extension install-file` or `--command extension build`); do not trust this paragraph.

Threading model (the part most likely to be wrong, so be careful):

- A background thread accepts connections and reads lines. It **never touches `bpy`**.
- Each request goes into a `queue.Queue` together with a `concurrent.futures.Future`-like object.
- A function `pump()` pops requests and executes them **on the main thread**. In the GUI it is driven by
  `bpy.app.timers.register(pump, persistent=True)` returning a small interval (e.g. 0.02 s). In headless mode (no
  event loop; timers do not fire in `--background`) a loop calls `pump()` directly. `pump()` is the single entry
  point so tests exercise the real path.
- Each handler runs inside try/except; exceptions become `blender_error`. A request that exceeds its budget returns
  `timeout`, but a running Python handler cannot be interrupted, so keep handlers short and say so in docs.
- Operators that need a 3D-viewport context (`bpy.ops.*` with `context.area`) need a context override
  (`bpy.context.temp_override`). **Prefer the data API (`bpy.data`, `obj.modifiers.new`, …) over `bpy.ops`** wherever
  possible; it is faster, context-free and testable headlessly. Use `bpy.ops` only when there is no data-API route and
  wrap it in a temp_override.
- Undo: wrap each mutating request so it creates one undo step with the tool name as its label (`bpy.ops.ed.undo_push`
  with a message). `scene.undo` / `scene.redo` tools use it.
- UI: a panel in the 3D viewport sidebar (N panel → "Omnix"): status, **Start/Stop**, "Allow run_python" checkbox
  (also in preferences), socket path, last request. Auto-start on load is a preference, default **off** for security
  (start must be an explicit click the first time).
- Registration must be clean: `unregister()` closes the socket, removes the file, unregisters timers, joins the thread
  with a timeout. Re-enabling the add-on must work without restarting Blender.

## 5. Headless mode (phase 2, same code)

`omnix-blender serve-headless` spawns `blender --background --factory-startup --python addon/headless_runner.py`,
which loads the same dispatcher, listens on a private socket, and loops `pump()`. The Rust server connects exactly as
for the GUI. This gives file-based automation without a GUI and is also what the integration tests use.

## 6. Rust server design

- Crate layout: `src/main.rs` (CLI), `src/mcp.rs` (tools), `src/link.rs` (socket client: connect, hello, request/response,
  timeouts, reconnect-on-demand), `src/blender.rs` (locate Blender, spawn headless, install add-on), `src/error.rs`.
- CLI: `omnix-blender serve` (default; stdio MCP), `doctor` (finds Blender, checks add-on installed and reachable,
  prints versions and the socket path, exits non-zero on problems), `install-addon` (installs the bundled add-on
  into the owner's Blender through Blender's own CLI, with a diff-free, idempotent, reversible procedure and a
  `--uninstall`), `serve-headless` (phase 2).
- The add-on source is embedded in the binary (`include_dir!`/`include_bytes!` of a zip built by `build.rs` or a
  committed script) so `cargo install` users get it with `install-addon`.
- **Lazy connection**: `serve` must answer the MCP handshake immediately and connect to Blender on first use, so the
  server starts fine when Blender is not running. If Blender is unreachable, tool calls return a clear actionable
  error ("Blender is not running or the add-on is not started: open Blender → N panel → Omnix → Start"), never hang.
- Tool annotations: query tools `readOnlyHint = true`; mutating tools `readOnlyHint = false`; destructive tools
  (`delete_objects`, `clear_scene`, `run_python`, `open_file` that discards unsaved work) `destructiveHint = true`.
  This is what lets omnix's read-only policy mode do the right thing, so get it right and test it.
- Outputs are compact JSON text (stable key order, rounded floats to 6 places). Keep results small: list tools take
  `limit`/`offset` and say when truncated. Do not dump meshes; return statistics.
- Never log or return absolute paths outside what the caller supplied. Log to stderr only (stdout is the MCP stream).

## 7. Tool set

Phase 1 (build these first, each with an argument schema, an annotation, and a test):

| Tool | Notes |
|---|---|
| `scene_info` | file path, unsaved?, units, frame range/current, render engine, counts of objects/materials/cameras/lights, active object, Blender version |
| `list_objects` | filter by type/collection/name glob; `limit`/`offset`; returns name, type, visible, parent, location |
| `get_object` | transform, parent/children, modifiers, material slots, mesh stats (verts/edges/faces), bounding box (world) |
| `create_primitive` | cube, uv_sphere, ico_sphere, cylinder, cone, torus, plane, circle, monkey; size, location, rotation, scale, name |
| `create_object` | empty, camera, light (point/sun/spot/area with energy/colour), text, curve |
| `set_transform` | absolute or relative; location/rotation (euler degrees)/scale; by name; supports several objects |
| `delete_objects` | by name list; reports what was removed; also purges orphan data optionally |
| `duplicate_object` / `set_parent` / `select_objects` | basic scene organisation; `collection` create/move/link |
| `set_material` | create or update a Principled BSDF material (base colour, metallic, roughness, emission, alpha) and assign to an object/slot |
| `add_modifier` / `apply_modifier` / `remove_modifier` | generic over modifier type with a validated property map (subsurf, bevel, mirror, array, boolean, solidify, decimate…) |
| `set_render_settings` | engine (CYCLES/BLENDER_EEVEE), resolution %, samples, output format/path |
| `render_image` | renders the current frame/camera; returns an MCP image (downscaled) + file path; optional `camera` and `samples` |
| `viewport_screenshot` | GUI only; returns an image of the active 3D viewport |
| `save_file` / `open_file` / `new_file` | `open_file` and `new_file` are destructive if there are unsaved changes: refuse unless `discard_changes:true` |
| `import_file` / `export_file` | fbx, obj, glb/gltf, usd(c/z), stl, ply, abc; uses the current importers (check what Blender 5.3 calls them) |
| `undo` / `redo` | N steps, returns the new undo-stack head if available |

Phase 2: animation (`insert_keyframe`, frame range, `set_frame`), shape keys, UV/unwrap, geometry-nodes input
parameters, collections/instancing, world/HDRI, `bake`, texture assignment from file, `set_property` (generic RNA
path set with type coercion and an allowlist), `list_addons`, headless mode, Windows TCP+token fallback.

`run_python` (always present in the tool list, disabled by default at the add-on): `{code: string, timeout_s?}` →
`{result?: json, stdout: string, error?: …}`. Runs with `bpy`, `bmesh`, `mathutils` available. It must capture stdout and
return the value of an optional `result` variable. Mark it destructive.

Also expose 2–3 MCP **resources** or a `tool` that returns a short "how to use these tools well" guide (units are
metres, Z-up, rotations in degrees in the API but radians in `bpy`, names are case-sensitive). Keep it under 400
words; models read it.

## 8. Milestones

| M | Deliverable | Done when |
|---|---|---|
| **M0** | Verify the unknowns in §10 empirically against the installed Blender 5.3, and write the findings to `docs/FINDINGS.md`. | Each open question has an answer backed by a command you ran. |
| **M1** | Repo skeleton, protocol, add-on core (socket thread, `pump()`, main-thread dispatch, hello), Rust `link` + `doctor` + MCP server with `scene_info` and `list_objects` only. | With Blender open and the add-on started, an MCP client can call `scene_info` and see the real scene. Headless test passes in CI-style (no GUI). |
| **M2** | All Phase-1 tools, undo steps, error codes, tool annotations. | Each tool has a headless integration test, plus Rust unit tests against a fake add-on. |
| **M3** | Security hardening + `install-addon`/`--uninstall` + packaging + docs + `run_python` gating. | Every item in §9 passes. `doctor` is green on the owner's machine. |
| **M4** | omnix integration check: `omnix add blender --command omnix-blender`, a read-only policy test, a code-mode `execute` example. | Documented and tested end to end. |
| **M5** | Phase-2 tools, headless mode, Linux CI. | |

Work in order. Commit locally after each milestone with a clear message. **Stop and report after M3**, then wait.

## 9. Test plan and acceptance criteria

1. **Rust unit tests**: protocol encode/decode, error mapping, timeouts, line-size limit, reconnect, schema validation
   (bad types rejected before reaching Blender), and the lazy-start behaviour when no socket exists.
2. **Fake add-on**: a tiny Rust test server speaking the protocol, so MCP-level tests run without Blender.
3. **Headless Blender integration tests** (the important ones): launch
   `/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup --python <runner>` with an isolated
   environment (`BLENDER_USER_RESOURCES=<tmp>`, `BLENDER_USER_SCRIPTS=<tmp>`, temp `HOME`-independent socket path) so
   **the owner's real Blender preferences and add-ons are never touched**. Skip these tests with a clear message when
   Blender is not found (`OMNIX_BLENDER_BIN` overrides discovery); they must not fail CI machines that lack Blender.
4. **Security tests**: socket dir/file modes; a second user cannot connect (assert mode bits); `run_python` returns
   `disabled` when the preference is off and works when on; oversized lines rejected; malformed JSON does not crash
   the add-on; the add-on does not bind any TCP port (`lsof`-style check or by construction).
5. **Lifecycle**: enable → start → stop → disable → enable again without restarting Blender; socket file removed on stop;
   stale socket file from a crashed run is detected and replaced.
6. **Annotations**: a test asserts every tool has the right `readOnlyHint`/`destructiveHint`, and an end-to-end test with
   omnix's `read_only` policy shows only the query tools are callable.
7. **Docs honesty**: README states what was tested (headless on Blender 5.3 / macOS arm64) and what was not.

## 10. Open questions to verify empirically in M0 (do not guess)

- Exact **extension manifest** schema and the correct CLI to install/build it on 5.3; whether a legacy `bl_info` add-on
  is still accepted (as a fallback path).
- Does `bpy.app.timers` run while the GUI is idle (it should) and at what interval; behaviour while a modal operator
  runs; behaviour when Blender is minimised (macOS App Nap can throttle timers; test it and document a workaround).
- Names/availability of importers/exporters in 5.3 (`bpy.ops.wm.obj_import`, `wm.usd_import`, `import_scene.fbx`,
  `export_scene.gltf`…) and whether they accept `filepath` the way the docs claim.
- Whether `render_image` can run from a timer callback without freezing the UI badly (use
  `bpy.ops.render.render(write_still=True)` with `INVOKE_DEFAULT` vs `EXEC_DEFAULT`; a synchronous render blocks the
  main thread, so long renders will make Blender unresponsive and requests queue behind them; document and set
  timeouts).
- `viewport_screenshot`: the supported way in 5.x (`bpy.ops.screen.screenshot_area` with an area override, or
  offscreen `gpu` rendering). Confirm it works headlessly (it will not; define the error).
- Undo-stack behaviour with `bpy.ops.ed.undo_push` from a timer, and whether data-API changes are undoable at all
  without an operator.
- Sandbox: how Blender 5.x treats `os`/`subprocess` inside `run_python` (it allows them; the gating is ours).
- Apple Silicon codesigning/quarantine concerns for Blender launched with `--python` from Rust (probably none).

## 11. Rules for the implementing agent

- This repo is **private and local**. No GitHub remote, no `gh repo create`, no publishing, no `cargo publish`.
- **Never modify the owner's real Blender preferences, add-ons or startup file** during development or tests. Use the
  isolated environment in §9.3. Installing the add-on into the real Blender is only done by the owner running
  `omnix-blender install-addon` (you may implement and test it against a temp `BLENDER_USER_RESOURCES`).
- Do not touch `~/project/omnix` except to read it. Do not touch other projects.
- Do not run destructive commands outside this directory and temp dirs.
- Prefer the data API over `bpy.ops`. Prefer small, tested steps. Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings`
  and the whole test suite before each commit.
- Be honest in docs and reports: say what is verified and what is not, including tools that only work in the GUI.
- When a plan statement here conflicts with what you observe in Blender 5.3, **trust the observation**, update
  `docs/FINDINGS.md`, and tell the owner.
