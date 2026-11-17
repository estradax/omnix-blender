"""Helpers shared by handlers. Runs on the main thread."""

import bpy

from ..protocol import INVALID_ARGUMENT, NOT_FOUND, OmnixError


def get_object(name):
    if not isinstance(name, str) or not name:
        raise OmnixError(INVALID_ARGUMENT, "object name must be a non-empty string")
    obj = bpy.data.objects.get(name)
    if obj is None:
        raise OmnixError(NOT_FOUND, "object '%s' does not exist (names are case-sensitive)" % name)
    return obj


def vec(v):
    return [float(x) for x in v]


def int_param(params, key, default, lo=0, hi=None):
    v = params.get(key, default)
    if isinstance(v, bool) or not isinstance(v, int):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be an integer" % key)
    if v < lo or (hi is not None and v > hi):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be between %s and %s" % (key, lo, hi if hi is not None else "inf"))
    return v


def object_visible(obj):
    try:
        return bool(obj.visible_get())
    except Exception:  # noqa: BLE001 - not in the active view layer
        return False


def vec3(params, key, default=None):
    """Read an optional 3-vector of numbers."""
    v = params.get(key, default)
    if v is None:
        return None
    if (
        not isinstance(v, (list, tuple))
        or len(v) != 3
        or any(isinstance(x, bool) or not isinstance(x, (int, float)) for x in v)
    ):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be an array of 3 numbers" % key)
    return [float(x) for x in v]


def color(params, key, default=None, n=(3, 4)):
    v = params.get(key, default)
    if v is None:
        return None
    if not isinstance(v, (list, tuple)) or len(v) not in n or any(
        isinstance(x, bool) or not isinstance(x, (int, float)) for x in v
    ):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be an array of %s numbers (0-1)" % (key, " or ".join(map(str, n))))
    return [min(1.0, max(0.0, float(x))) for x in v]


def number(params, key, default=None, lo=None, hi=None):
    v = params.get(key, default)
    if v is None:
        return None
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be a number" % key)
    if (lo is not None and v < lo) or (hi is not None and v > hi):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be between %s and %s" % (key, lo, hi))
    return float(v)


def string(params, key, default=None, required=False):
    v = params.get(key, default)
    if v is None:
        if required:
            raise OmnixError(INVALID_ARGUMENT, "'%s' is required" % key)
        return None
    if not isinstance(v, str) or not v:
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be a non-empty string" % key)
    return v


def boolean(params, key, default=False):
    v = params.get(key, default)
    if not isinstance(v, bool):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be true or false" % key)
    return v


def name_list(params, key="names"):
    v = params.get(key)
    if not isinstance(v, list) or not v or any(not isinstance(x, str) or not x for x in v):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be a non-empty array of object names" % key)
    return v


def get_objects(names):
    """Resolve all names or raise not_found listing every missing one (nothing is changed on failure)."""
    missing = [n for n in names if n not in bpy.data.objects]
    if missing:
        raise OmnixError(NOT_FOUND, "object(s) not found (names are case-sensitive): %s" % ", ".join(missing))
    return [bpy.data.objects[n] for n in names]


def enum_choice(value, choices, what):
    if not isinstance(value, str) or value.upper() not in choices:
        raise OmnixError(INVALID_ARGUMENT, "%s must be one of: %s" % (what, ", ".join(sorted(choices))))
    return value.upper()


def target_collection(name=None):
    if name is None:
        return bpy.context.view_layer.active_layer_collection.collection
    coll = bpy.data.collections.get(name)
    if coll is None:
        raise OmnixError(NOT_FOUND, "collection '%s' does not exist (create it with the collection tool)" % name)
    return coll


def link_new_object(obj, collection=None):
    target_collection(collection).objects.link(obj)
    return obj


def unsaved_changes(ctx):
    """Best effort. In the GUI Blender's own dirty flag is used; headless it was observed to be unreliable
    (docs/FINDINGS.md), so the add-on tracks changes made through its own mutating methods."""
    if bpy.app.background:
        return bool(ctx.dirty)
    return bool(bpy.data.is_dirty) or bool(ctx.dirty)
