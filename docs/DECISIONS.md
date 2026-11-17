# Decisions and deviations from PLAN.md

Reasons are in docs/FINDINGS.md where an observation drove the change.

| Topic | Decision |
|---|---|
| hello | The client sends `hello` first (the add-on never speaks unsolicited). Anything else before `hello` is `protocol_error` and closes the connection. |
| Error codes | PLAN's enum plus `protocol_error` (malformed JSON, missing hello). Rust adds client-side codes `not_connected`, `io_error`, `version_mismatch`, `bad_socket_path`. |
| Envelope | Optional top-level `timeout_s` per request. A second request while one is outstanding returns `busy` (v1 is serial). |
| Socket path length | Both sides reject paths over 103 bytes (macOS `sun_path` limit, measured). |
| Extension install | `extension install-file -r user_default -e`; success is judged from the `STATUS` line because the exit code is 0 on failure (measured). Add-on embedded by `build.rs`, zipped at install time. |
| Tests load the add-on | `--factory-startup` ignores installed extensions, so most e2e tests use `addon/headless_runner.py`; the install/lifecycle/doctor tests use the really installed extension in a temp profile. `headless.shutdown` exists only in the runner. |
| Undo | Mutating handlers push one undo step *after* the change (observed semantics). The add-on also pushes a step at start and after open/new so the first change is undoable. `undo`/`redo`/`select_objects`/file tools do not push. `run_python` accepts `undo:false` (no step, no dirty flag; used by tests for inspection). |
| Dirty tracking | `bpy.data.is_dirty` was unreliable headless. Headless: the add-on's own flag; GUI: `is_dirty` OR the flag. Best effort, documented. |
| Primitives | Built with `bmesh` + data API (no `bpy.ops`, no context needed). `size` is overall extent in metres (cube/plane edge, diameter for round shapes). `apply_modifier`, imports/exports, undo/redo, render and screenshot still use operators (no data-API route) with a context override where needed. |
| Annotations | Additive tools: `readOnly=false, destructive=false`; delete/apply/remove modifier, save/open/new/export, run_python: destructive. `render_image` is *not* read-only (CPU heavy, blocks the UI, writes files). `select_objects`, `undo`, `redo` are non-read-only, non-destructive. |
| Unknown tool args | Rejected (`deny_unknown_fields`) so typos do not silently change meaning. |
| Paths in output | `scene_info` returns the .blend path and renders return the temp image path, because the tool contract needs them; nothing else leaks absolute paths. |
| Not done (M4+) | `serve-headless` CLI, Linux CI, phase-2 tools. |

## Review follow-up (two independent reviews)

| Topic | Decision |
|---|---|
| open_file | `use_scripts=False` always. Observed before the fix: with `use_scripts_auto_execute=True` an auto-run text block executed during `open_file`. |
| File policy | See SECURITY.md. Violations are code `disabled` (outside roots / hidden / protected) or `invalid_argument` (overwrite, extension/format). Operations use the *resolved* path. `scene_info.file` and tool results therefore show resolved paths (`/private/tmp/...` on macOS). Policy is checked before existence so errors do not reveal files outside the roots. |
| Allowed roots | Saved .blend folder, preference list, render temp dir. Preference list is a `CollectionProperty` edited in the add-on preferences; headless runner takes `--allowed-folder`, `--allow-outside-files`. |
| Protected list | Includes `/usr`, `/bin`, `/sbin` beyond the requested set (nobody should export there). |
| Socket directory | Refuse (not chmod) any group/other bits and any symlink; documented in SECURITY.md. |
| Stale sockets | Unlink only on ECONNREFUSED. Everything else refuses and keeps the file. On macOS a live listener with a full backlog may report ECONNREFUSED instead of timing out; that case cannot be distinguished from a dead socket by a probe and was not reproduced. |
| Params validation | Each add-on method declares accepted `params` keys (`@method(..., keys=)`), advertised in `hello.params`; a test checks every MCP tool property is accepted. Unknown top-level request fields are a `protocol_error`. |
| stop() | Bounded to ~1.5 s total; waiting connection threads notice the stop within 0.1 s. |
| viewport_screenshot | Stays `readOnlyHint=true`: it changes no scene state and writes only a private temp image (the same side effect class as a cache), unlike `render_image` which is CPU heavy, blocks the UI and is therefore not marked read-only. |
| Test strictness | `OMNIX_BLENDER_REQUIRE=1` turns Blender/lsof skips into failures. |
