"""Read-only scene queries."""

import fnmatch

import bpy

from ..dispatch import method
from ..protocol import INVALID_ARGUMENT, NOT_FOUND, OmnixError
from .common import int_param, object_visible, unsaved_changes, vec


@method("status")
def status(params, ctx):
    return {
        "blender": bpy.app.version_string,
        "headless": bool(bpy.app.background),
        "run_python_enabled": bool(ctx.allow_run_python()),
    }


@method("scene_info")
def scene_info(params, ctx):
    sc = bpy.context.scene
    active = bpy.context.view_layer.objects.active
    return {
        "blender": bpy.app.version_string,
        "scene": sc.name,
        "file": bpy.data.filepath or None,
        "unsaved": unsaved_changes(ctx) or not bpy.data.filepath,
        "units": {
            "system": sc.unit_settings.system,
            "scale_length": sc.unit_settings.scale_length,
            "length_unit": sc.unit_settings.length_unit,
            "rotation": sc.unit_settings.system_rotation,
        },
        "frame": {"start": sc.frame_start, "end": sc.frame_end, "current": sc.frame_current, "fps": sc.render.fps},
        "render_engine": sc.render.engine,
        "counts": {
            "objects": len(sc.objects),
            "meshes": len(bpy.data.meshes),
            "materials": len(bpy.data.materials),
            "cameras": sum(1 for o in sc.objects if o.type == "CAMERA"),
            "lights": sum(1 for o in sc.objects if o.type == "LIGHT"),
            "collections": len(bpy.data.collections),
        },
        "active_object": active.name if active else None,
        "selected": sorted(o.name for o in sc.objects if o.select_get()),
        "camera": sc.camera.name if sc.camera else None,
    }


@method("list_objects")
def list_objects(params, ctx):
    limit = int_param(params, "limit", 100, 1, 1000)
    offset = int_param(params, "offset", 0, 0)
    otype = params.get("type")
    if otype is not None and not isinstance(otype, str):
        raise OmnixError(INVALID_ARGUMENT, "'type' must be a string such as MESH, CAMERA, LIGHT")
    glob = params.get("name_glob")
    if glob is not None and not isinstance(glob, str):
        raise OmnixError(INVALID_ARGUMENT, "'name_glob' must be a string")
    coll_name = params.get("collection")
    objs = bpy.context.scene.objects
    if coll_name is not None:
        coll = bpy.data.collections.get(coll_name)
        if coll is None:
            raise OmnixError(NOT_FOUND, "collection '%s' does not exist" % coll_name)
        objs = coll.all_objects
    rows = [
        o
        for o in objs
        if (otype is None or o.type == otype.upper()) and (glob is None or fnmatch.fnmatchcase(o.name, glob))
    ]
    rows.sort(key=lambda o: o.name)
    total = len(rows)
    page = rows[offset : offset + limit]
    return {
        "total": total,
        "offset": offset,
        "limit": limit,
        "truncated": offset + len(page) < total,
        "objects": [
            {
                "name": o.name,
                "type": o.type,
                "visible": object_visible(o),
                "parent": o.parent.name if o.parent else None,
                "location": vec(o.location),
            }
            for o in page
        ],
    }
