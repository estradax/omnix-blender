# M0 findings (Blender 5.3.0, macOS 15.7 arm64, Python 3.13.13 inside Blender)

Every answer below comes from a command run against `/Applications/Blender.app` on 2026-11-17/18, always with
`--factory-startup` and `BLENDER_USER_RESOURCES` / `BLENDER_USER_SCRIPTS` pointing at a fresh temp directory.
Probe scripts were throwaway files in that temp dir. Items marked **NOT VERIFIED** could not be tested without a human.

## Summary of deviations from PLAN.md

| PLAN.md says | Observed | What we do |
|---|---|---|
| `install-file` exit code signals failure | Exit code is **0** even for a missing zip or unknown repo. Only `remove` of a missing package exits 1. | Parse the `STATUS`/`ERROR` lines, not the exit code. |
| Plan implies `--factory-startup` runs can load the add-on | `--factory-startup` ignores user prefs, so an installed+enabled extension is **not** loaded. | Tests load the add-on via `headless_runner.py` (adds the add-on dir to `sys.path`). |
| Unix socket path is `<config_dir>/omnix-blender/blender.sock` | `AF_UNIX` paths are limited to **103 bytes** on macOS (bind fails at 104). | Both sides check the length and fail with a clear message; tests use short `/tmp/...` paths. |
| `bpy.data.is_dirty` can detect unsaved work | Unreliable: stayed `False` after adding objects via data API and flipped to `True` only after an `undo_push`. | `open_file`/`new_file` use `is_dirty`, but docs say it is best-effort. |
| Render engine list | In 5.3 the EEVEE identifier is `BLENDER_EEVEE` (only that one shows in the static enum), `CYCLES` and `BLENDER_WORKBENCH` also assign fine. | Allow `BLENDER_EEVEE`, `CYCLES`, `BLENDER_WORKBENCH`. |

## 1. Extension manifest, install CLI, legacy fallback

- `blender --command extension` subcommands: `server-generate build validate list sync update install install-file remove repo-list repo-info repo-add repo-remove`.
- Minimal manifest that validated, built and installed:
  ```toml
  schema_version = "1.0.0"
  id = "omnix_blender"
  version = "0.1.0"
  name = "Omnix Blender"
  tagline = "MCP bridge for Omnix"
  maintainer = "Omnix <noreply@example.com>"
  type = "add-on"
  blender_version_min = "4.2.0"
  license = ["SPDX:MIT"]
  ```
  No `permissions` key is in the manifest above and validate/build/install accepted it; whether the `permissions`
  table is required for network/file access by add-ons was not tested (our add-on only uses a local Unix socket).
- `extension validate <dir>` printed "Success parsing TOML"; `extension build --source-dir D --output-dir O` produced
  `omnix_blender-0.1.0.zip`.
- Install: `extension install-file -r user_default -e FILE.zip` -> `STATUS Installed "omnix_blender"`; running it again
  -> `STATUS Reinstalled "omnix_blender"` (idempotent). `-e` enables it in the (isolated) `userpref.blend`
  (`$BLENDER_USER_RESOURCES/config/userpref.blend`). Files land in
  `$BLENDER_USER_RESOURCES/extensions/user_default/omnix_blender/`. The module name is
  `bl_ext.user_default.omnix_blender`.
- Remove: `extension remove omnix_blender` -> `STATUS Removed "omnix_blender"`, and the add-on was gone from the prefs
  afterwards. Removing a missing package prints `Package ... not installed` and exits 1. A harmless
  `[Errno 2] ... extensions/blender_org` line is printed in a fresh resource dir.
- `--factory-startup` does NOT load the enabled extension (add-on list shows only the built-in ones); a run without it
  does (`OMNIX registered`). `addon_utils.enable('bl_ext.user_default.omnix_blender')` also works.
- Legacy `bl_info` single-file add-on in `$BLENDER_USER_SCRIPTS/addons/legacy_x.py` is still accepted
  (`addon_utils.enable('legacy_x')` succeeded). Kept as a documented fallback only.

## 2. Timers

- `bpy.app.timers` does **not** fire in `--background` (0 ticks over 0.5 s of sleeping). Headless must call `pump()` in a loop.
- In the GUI (`blender --factory-startup --python g1.py`, window open, idle) a timer returning `0.02` ticked at
  42 Hz (mean gap 23.6 ms, max gap 37 ms). `pump()` on a 0.02 s timer is fine.
- **NOT VERIFIED**: behaviour while a modal operator runs, while minimised, or under macOS App Nap. Needs a human.
  Mitigation if timers throttle: the add-on's panel shows "last request" time, and docs recommend keeping Blender
  visible; a `caffeinate`-style workaround is not implemented.

## 3. Importers / exporters (all round-tripped a cube with `filepath=`)

All succeeded (`{'FINISHED'}`, file written, 1 object imported) headlessly:
`wm.obj_import/export`, `wm.stl_import/export`, `wm.ply_import/export`, `wm.usd_import/export` (.usdc),
`wm.alembic_import/export`, `import_scene.fbx`/`export_scene.fbx`, `import_scene.gltf`/`export_scene.gltf` (.glb),
also present: `wm.collada_import/export`. FBX and glTF are bundled add-ons enabled by default (`io_scene_fbx`,
`io_scene_gltf2`); under `--factory-startup` they are enabled, so they work. Note the old `import_scene.obj` /
`import_mesh.stl` names were not probed.

## 4. Rendering

- `bpy.ops.render.render(write_still=True)` (EXEC_DEFAULT) works headlessly for EEVEE (5.0 s first time, shader
  compile), CYCLES CPU 4 samples (0.4 s) and WORKBENCH (0.04 s) at 320x240.
- It also works **from a timer callback in the GUI** (workbench, 160x120). It is synchronous: it blocks the main thread
  (and the UI) for the whole render. `INVOKE_DEFAULT` was not tried; we use EXEC and document the freeze.

## 5. Viewport screenshot

- Headless: `bpy.ops.screen.screenshot_area` fails with `poll() failed, context is incorrect`. Error to define:
  `unsupported`, "viewport_screenshot needs the Blender GUI".
- GUI, from a timer, with `bpy.context.temp_override(window=win, area=<VIEW_3D area>, region=<WINDOW region>)`:
  `{'FINISHED'}`, PNG written (1.37 MB). Works.

## 6. Undo

- `bpy.ops.ed.undo_push(message=...)` works from a timer in the GUI and in background mode.
- Semantics observed (background): a push records the state **after** the change. `ed.undo()` goes back to the previous
  push, so "mutate, then push" gives one undo step per request. Data-API changes (`bpy.data.objects.new` + link) are
  undone this way, but only if a push follows them; changes made after the last push are lost on undo.
- `ed.redo()` restored the object. Both returned `{'FINISHED'}`.
- **NOT VERIFIED**: `ed.undo()` from a GUI timer (only `undo_push` was tried in the GUI). Will be smoke-tested in M2.

## 7. Sandbox / run_python

- `os`, `subprocess` work inside Blender Python (`subprocess.run(['echo','hi'])` -> `hi`). The only gate is ours.
- Capturing output: `contextlib.redirect_stdout` + `exec(code, globals)` works; `result` can be read from the globals.
- `bpy.context.preferences.filepaths.use_scripts_auto_execute` is False under factory settings (irrelevant to us).
- A background thread calling `bpy.data.objects.new` did not crash in headless mode, but that is undefined behaviour.
  The design keeps `bpy` strictly on the main thread.
- Wrong operator argument types raise `TypeError` (becomes `blender_error`).

## 8. Apple Silicon / quarantine

- `/Applications/Blender.app` carries a `com.apple.quarantine` attribute (from a browser download) but is signed
  (`org.blenderfoundation.blender`, arm64). Launching its inner binary with `--python` from the shell worked with no
  prompt. Launching from a Rust `Command` is the same exec path; not separately tested before M1.

## 9. Other observations

- Quitting the GUI via `bpy.ops.wm.quit_blender()` wrote a recovery `quit.blend` into the system temp dir.

## Follow-up observations (review fixes)

- `bpy.ops.wm.open_mainfile` honours the profile's `use_scripts_auto_execute`: with it enabled, a text block with
  `use_module=True` ran during the load (reproduced headless); `use_scripts=False` prevents it.
- Exporters (`wm.obj_export`) write side files next to the target (`.mtl`); they follow symlinks.
- `tempfile.Builder::tempdir_in` creates directories with the umask-dependent mode (0755 here), not 0700; tests chmod explicitly.
- macOS `/tmp` is a symlink to `/private/tmp`; resolved paths therefore differ from the paths callers pass.
