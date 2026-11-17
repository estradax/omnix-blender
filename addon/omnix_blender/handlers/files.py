"""File operations, import/export and undo/redo."""

import os

import bpy

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


def _abs(path):
    return os.path.abspath(os.path.expanduser(path))


@method("save_file", mutating=False)
def save_file(params, ctx):
    path = string(params, "path")
    if path is None:
        if not bpy.data.filepath:
            raise OmnixError(INVALID_ARGUMENT, "the scene has never been saved; pass 'path' (e.g. /Users/me/scene.blend)")
        bpy.ops.wm.save_mainfile()
        ctx.dirty = False
        return {"file": bpy.data.filepath}
    path = _abs(path)
    if not path.lower().endswith(".blend"):
        path += ".blend"
    if not os.path.isdir(os.path.dirname(path)):
        raise OmnixError(NOT_FOUND, "directory does not exist: %s" % os.path.dirname(path))
    bpy.ops.wm.save_as_mainfile(filepath=path)
    ctx.dirty = False
    return {"file": bpy.data.filepath}


@method("open_file")
def open_file(params, ctx):
    path = _abs(string(params, "path", required=True))
    if not os.path.isfile(path):
        raise OmnixError(NOT_FOUND, "file does not exist: %s" % path)
    _require_discard(params, "open_file", ctx)
    bpy.ops.wm.open_mainfile(filepath=path, load_ui=False)
    ctx.dirty = False
    _initial_undo_step()
    return {"file": bpy.data.filepath, "objects": len(bpy.context.scene.objects)}


@method("new_file")
def new_file(params, ctx):
    _require_discard(params, "new_file", ctx)
    bpy.ops.wm.read_homefile(use_empty=boolean(params, "empty", False), load_ui=False)
    ctx.dirty = False
    _initial_undo_step()
    return {"file": None, "objects": len(bpy.context.scene.objects)}


def _format_of(path, fmt):
    if fmt is None:
        fmt = EXT.get(os.path.splitext(path)[1].lower())
        if fmt is None:
            raise OmnixError(INVALID_ARGUMENT, "cannot infer the format from '%s'; pass 'format' (one of %s)" % (path, ", ".join(sorted(FORMATS))))
    fmt = fmt.lower()
    fmt = {"glb": "gltf", "usdc": "usd", "usda": "usd", "usdz": "usd"}.get(fmt, fmt)
    if fmt not in FORMATS:
        raise OmnixError(INVALID_ARGUMENT, "unsupported format '%s'; supported: %s" % (fmt, ", ".join(sorted(FORMATS))))
    return fmt


def _op(module, name):
    return getattr(getattr(bpy.ops, module), name)


@method("import_file", mutating=True)
def import_file(params, ctx):
    path = _abs(string(params, "path", required=True))
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


@method("export_file")
def export_file(params, ctx):
    path = _abs(string(params, "path", required=True))
    fmt = _format_of(path, params.get("format"))
    if not os.path.isdir(os.path.dirname(path)):
        raise OmnixError(NOT_FOUND, "directory does not exist: %s" % os.path.dirname(path))
    _, _, module, exp, sel = FORMATS[fmt]
    kwargs = {"filepath": path}
    if boolean(params, "selected_only", False):
        kwargs[sel] = True
    try:
        res = _op(module, exp)(**kwargs)
    except RuntimeError as exc:
        raise OmnixError(INVALID_ARGUMENT, "export failed: %s" % str(exc).strip())
    if "FINISHED" not in res or not os.path.exists(path):
        raise OmnixError("blender_error", "export did not produce a file (%s)" % sorted(res))
    return {"format": fmt, "path": path, "bytes": os.path.getsize(path)}


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


@method("undo")
def undo(params, ctx):
    return _step(bpy.ops.ed.undo, params)


@method("redo")
def redo(params, ctx):
    return _step(bpy.ops.ed.redo, params)
