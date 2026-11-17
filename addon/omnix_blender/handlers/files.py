"""File operations, import/export and undo/redo."""

import os

import bpy

from .. import pathpolicy
from ..dispatch import method
from ..protocol import INVALID_ARGUMENT, NOT_FOUND, OmnixError
from .common import boolean, int_param, string, unsaved_changes

# format -> (import module, import op, export module, export op, kwarg name for "selected only" on export)
FORMATS = {
    "obj": ("wm", "obj_import", "wm", "obj_export", "export_selected_objects"),
    "stl": ("wm", "stl_import", "wm", "stl_export", "export_selected_objects"),
    "ply": ("wm", "ply_import", "wm", "ply_export", "export_selected_objects"),
    "usd": ("wm", "usd_import", "wm", "usd_export", "selected_objects_only"),
    "abc": ("wm", "alembic_import", "wm", "alembic_export", "selected"),
    "fbx": ("import_scene", "fbx", "export_scene", "fbx", "use_selection"),
    "gltf": ("import_scene", "gltf", "export_scene", "gltf", "use_selection"),
}
EXT = {
    ".obj": "obj", ".stl": "stl", ".ply": "ply", ".usd": "usd", ".usdc": "usd", ".usda": "usd", ".usdz": "usd",
    ".abc": "abc", ".fbx": "fbx", ".gltf": "gltf", ".glb": "gltf",
}




def _require_discard(params, what, ctx):
    if unsaved_changes(ctx) and not boolean(params, "discard_changes", False):
        raise OmnixError(
            INVALID_ARGUMENT,
            "the open file has unsaved changes and %s would discard them. Call save_file first, or pass discard_changes=true. "
            "(Dirty-tracking is best effort: Blender may not flag changes made through Python.)" % what,
        )


def _initial_undo_step():
    """Loading a file resets the undo stack; push the loaded state so the first change is undoable."""
    try:
        bpy.ops.ed.undo_push(message="Omnix: loaded")
    except Exception:  # noqa: BLE001
        pass


def _same_file(a, b):
    return bool(a) and bool(b) and os.path.realpath(a) == os.path.realpath(b)


@method("save_file", mutating=False, keys=("path", "overwrite", ))
def save_file(params, ctx):
    raw = string(params, "path")
    overwrite = boolean(params, "overwrite", False)
    if raw is None:
        if not bpy.data.filepath:
            raise OmnixError(INVALID_ARGUMENT, "the scene has never been saved; pass 'path' (e.g. /Users/me/scene.blend)")
        pathpolicy.resolve(bpy.data.filepath, ctx, "current file")
        bpy.ops.wm.save_mainfile()
        ctx.dirty = False
        return {"file": bpy.data.filepath}
    if not raw.lower().endswith(".blend"):
        raw += ".blend"
    path = pathpolicy.resolve(raw, ctx, "path")
    if not os.path.isdir(os.path.dirname(path)):
        raise OmnixError(NOT_FOUND, "directory does not exist: %s" % os.path.dirname(path))
    if not _same_file(path, bpy.data.filepath):  # saving over the open file in place is the normal workflow
        pathpolicy.require_writable_target(path, overwrite)
    bpy.ops.wm.save_as_mainfile(filepath=path)
    ctx.dirty = False
    return {"file": bpy.data.filepath}


@method("open_file", keys=("path", "discard_changes", ))
def open_file(params, ctx):
    path = pathpolicy.resolve(string(params, "path", required=True), ctx, "path")
    if not os.path.isfile(path):
        raise OmnixError(NOT_FOUND, "file does not exist: %s" % path)
    _require_discard(params, "open_file", ctx)
    # use_scripts=False: never run scripts embedded in the file (text blocks marked "register", drivers), even if
    # the user's preferences trust auto-execution. A .blend from an untrusted source must not become code execution.
    bpy.ops.wm.open_mainfile(filepath=path, load_ui=False, use_scripts=False)
    ctx.dirty = False
    _initial_undo_step()
    return {"file": bpy.data.filepath, "objects": len(bpy.context.scene.objects)}


@method("new_file", keys=("discard_changes", "empty", ))
def new_file(params, ctx):
    _require_discard(params, "new_file", ctx)
    bpy.ops.wm.read_homefile(use_empty=boolean(params, "empty", False), load_ui=False)
    ctx.dirty = False
    _initial_undo_step()
    return {"file": None, "objects": len(bpy.context.scene.objects)}


def _format_of(path, fmt):
    """The format for ``path``. The extension must be known and must agree with an explicit ``format``."""
    ext = os.path.splitext(path)[1].lower()
    from_ext = EXT.get(ext)
    if from_ext is None:
        raise OmnixError(
            INVALID_ARGUMENT,
            "unsupported file extension '%s'; supported: %s" % (ext or "(none)", ", ".join(sorted(EXT))),
        )
    if fmt is None:
        return from_ext
    if not isinstance(fmt, str):
        raise OmnixError(INVALID_ARGUMENT, "'format' must be a string")
    given = {"glb": "gltf", "usdc": "usd", "usda": "usd", "usdz": "usd"}.get(fmt.lower(), fmt.lower())
    if given not in FORMATS:
        raise OmnixError(INVALID_ARGUMENT, "unsupported format '%s'; supported: %s" % (fmt, ", ".join(sorted(FORMATS))))
    if given != from_ext:
        raise OmnixError(INVALID_ARGUMENT, "format '%s' contradicts the file extension '%s' (which means '%s')" % (fmt, ext, from_ext))
    return given


def _op(module, name):
    return getattr(getattr(bpy.ops, module), name)


@method("import_file", mutating=True, keys=("path", "format", ))
def import_file(params, ctx):
    path = pathpolicy.resolve(string(params, "path", required=True), ctx, "path")
    if not os.path.isfile(path):
        raise OmnixError(NOT_FOUND, "file does not exist: %s" % path)
    fmt = _format_of(path, params.get("format"))
    module, imp, _, _, _ = FORMATS[fmt]
    before = set(bpy.data.objects.keys())
    try:
        res = _op(module, imp)(filepath=path)
    except RuntimeError as exc:
        raise OmnixError(INVALID_ARGUMENT, "import failed: %s" % str(exc).strip())
    if "FINISHED" not in res:
        raise OmnixError("blender_error", "import did not finish (%s)" % sorted(res))
    new = sorted(set(bpy.data.objects.keys()) - before)
    return {"format": fmt, "imported": new, "count": len(new)}


def _expected_side_files(path, fmt):
    stem, ext = os.path.splitext(path)
    if fmt == "obj":
        return [stem + ".mtl"]
    if fmt == "gltf" and ext.lower() == ".gltf":
        return [stem + ".bin"]
    return []


def _snapshot(directory):
    out = {}
    try:
        with os.scandir(directory) as it:
            for i, e in enumerate(it):
                if i >= 5000:
                    break
                try:
                    st = e.stat(follow_symlinks=False)
                    out[e.name] = (st.st_mtime_ns, st.st_size)
                except OSError:
                    pass
    except OSError:
        pass
    return out


@method("export_file", keys=("path", "format", "selected_only", "overwrite", ))
def export_file(params, ctx):
    raw = string(params, "path", required=True)
    overwrite = boolean(params, "overwrite", False)
    path = pathpolicy.resolve(raw, ctx, "path")
    fmt = _format_of(path, params.get("format"))
    directory = os.path.dirname(path)
    if not os.path.isdir(directory):
        raise OmnixError(NOT_FOUND, "directory does not exist: %s" % directory)
    pathpolicy.require_writable_target(path, overwrite)
    # Files the exporter writes next to the main file get the same checks (policy, symlinks, overwrite).
    for side in _expected_side_files(path, fmt):
        real_side = pathpolicy.resolve(side, ctx, "side file")
        pathpolicy.require_writable_target(real_side, overwrite, "side file")
    _, _, module, exp, sel = FORMATS[fmt]
    kwargs = {"filepath": path}
    if boolean(params, "selected_only", False):
        kwargs[sel] = True
    before = _snapshot(directory)
    try:
        res = _op(module, exp)(**kwargs)
    except RuntimeError as exc:
        raise OmnixError(INVALID_ARGUMENT, "export failed: %s" % str(exc).strip())
    if "FINISHED" not in res or not os.path.exists(path):
        raise OmnixError("blender_error", "export did not produce a file (%s)" % sorted(res))
    after = _snapshot(directory)
    main = os.path.basename(path)
    side_files = sorted(
        os.path.join(directory, n) for n, sig in after.items() if n != main and before.get(n) != sig
    )
    return {"format": fmt, "path": path, "bytes": os.path.getsize(path), "side_files": side_files}


def _step(op, params):
    n = int_param(params, "steps", 1, 1, 100)
    done = 0
    for _ in range(n):
        try:
            if "FINISHED" not in op():
                break
        except RuntimeError:
            break  # nothing left to undo/redo
        done += 1
    return {"steps_requested": n, "steps_done": done, "objects": len(bpy.context.scene.objects)}


@method("undo", keys=("steps", ))
def undo(params, ctx):
    return _step(bpy.ops.ed.undo, params)


@method("redo", keys=("steps", ))
def redo(params, ctx):
    return _step(bpy.ops.ed.redo, params)
