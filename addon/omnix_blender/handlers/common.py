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
