"""Object queries and scene-organisation handlers. Data API first; no bpy.ops except where noted."""

import math

import bmesh
import bpy
from mathutils import Matrix, Vector

from ..dispatch import method
from ..protocol import INVALID_ARGUMENT, OmnixError
from .common import (
    boolean,
    color,
    enum_choice,
    get_object,
    get_objects,
    link_new_object,
    name_list,
    number,
    object_visible,
    string,
    target_collection,
    vec,
    vec3,
)

PRIMITIVES = {"CUBE", "UV_SPHERE", "ICO_SPHERE", "CYLINDER", "CONE", "TORUS", "PLANE", "CIRCLE", "MONKEY"}
OBJECT_KINDS = {"EMPTY", "CAMERA", "LIGHT", "TEXT", "CURVE"}
LIGHT_TYPES = {"POINT", "SUN", "SPOT", "AREA"}


def describe(obj):
    return {
        "name": obj.name,
        "type": obj.type,
        "location": vec(obj.location),
        "rotation_deg": [math.degrees(a) for a in obj.rotation_euler],
        "scale": vec(obj.scale),
    }


def world_bbox(obj):
    if obj.type in {"MESH", "CURVE", "SURFACE", "FONT", "META"} or obj.bound_box:
        pts = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
        if pts and any(p.length for p in pts):
            lo = [min(p[i] for p in pts) for i in range(3)]
            hi = [max(p[i] for p in pts) for i in range(3)]
            return {"min": lo, "max": hi}
    loc = obj.matrix_world.translation
    return {"min": vec(loc), "max": vec(loc)}


@method("get_object")
def get_object_info(params, ctx):
    obj = get_object(params.get("name"))
    out = describe(obj)
    out.update(
        {
            "world_location": vec(obj.matrix_world.translation),
            "dimensions": vec(obj.dimensions),
            "visible": object_visible(obj),
            "selected": obj.select_get(),
            "parent": obj.parent.name if obj.parent else None,
            "children": sorted(c.name for c in obj.children),
            "collections": sorted(c.name for c in obj.users_collection),
            "data": obj.data.name if obj.data else None,
            "modifiers": [{"name": m.name, "type": m.type, "show_viewport": m.show_viewport} for m in obj.modifiers],
            "material_slots": [s.material.name if s.material else None for s in obj.material_slots],
            "bounding_box_world": world_bbox(obj),
        }
    )
    if obj.type == "MESH":
        me = obj.data
        out["mesh"] = {"vertices": len(me.vertices), "edges": len(me.edges), "faces": len(me.polygons)}
    if obj.type == "LIGHT":
        out["light"] = {"type": obj.data.type, "energy": obj.data.energy, "color": vec(obj.data.color)}
    if obj.type == "CAMERA":
        out["camera"] = {"lens": obj.data.lens, "type": obj.data.type}
    return out


def _apply_placement(obj, params):
    loc = vec3(params, "location")
    rot = vec3(params, "rotation")
    scale = vec3(params, "scale")
    if loc:
        obj.location = loc
    if rot:
        obj.rotation_euler = [math.radians(a) for a in rot]
    if scale:
        obj.scale = scale


def _build_mesh(kind, size, segments):
    bm = bmesh.new()
    r = size / 2.0
    if kind == "CUBE":
        bmesh.ops.create_cube(bm, size=size)
    elif kind == "PLANE":
        bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=r)
    elif kind == "UV_SPHERE":
        bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=max(3, segments // 2), radius=r)
    elif kind == "ICO_SPHERE":
        bmesh.ops.create_icosphere(bm, subdivisions=2, radius=r)
    elif kind == "CYLINDER":
        bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segments, radius1=r, radius2=r, depth=size)
    elif kind == "CONE":
        bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segments, radius1=r, radius2=0.0, depth=size)
    elif kind == "CIRCLE":
        bmesh.ops.create_circle(bm, cap_ends=True, cap_tris=False, segments=segments, radius=r)
    elif kind == "MONKEY":
        bmesh.ops.create_monkey(bm)
        bmesh.ops.scale(bm, vec=(r, r, r), verts=bm.verts)
    elif kind == "TORUS":
        major, minor = r * 0.75, r * 0.25
        minor_segs = max(3, segments // 2)
        verts = []
        for i in range(segments):
            a = 2 * math.pi * i / segments
            ring = []
            for j in range(minor_segs):
                b = 2 * math.pi * j / minor_segs
                rr = major + minor * math.cos(b)
                ring.append(bm.verts.new((rr * math.cos(a), rr * math.sin(a), minor * math.sin(b))))
            verts.append(ring)
        for i in range(segments):
            for j in range(minor_segs):
                i2, j2 = (i + 1) % segments, (j + 1) % minor_segs
                bm.faces.new((verts[i][j], verts[i2][j], verts[i2][j2], verts[i][j2]))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return bm


@method("create_primitive", mutating=True)
def create_primitive(params, ctx):
    kind = enum_choice(params.get("type"), PRIMITIVES, "'type'")
    size = number(params, "size", 2.0, lo=1e-6, hi=1e6)
    segments = int(number(params, "segments", 32, lo=3, hi=256))
    name = string(params, "name", kind.title().replace("_", ""))
    bm = _build_mesh(kind, size, segments)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    link_new_object(obj, params.get("collection"))
    _apply_placement(obj, params)
    return describe(obj)


@method("create_object", mutating=True)
def create_object(params, ctx):
    kind = enum_choice(params.get("type"), OBJECT_KINDS, "'type'")
    name = string(params, "name", kind.title())
    if kind == "EMPTY":
        obj = bpy.data.objects.new(name, None)
        obj.empty_display_size = number(params, "size", 1.0, lo=1e-6)
    elif kind == "CAMERA":
        cam = bpy.data.cameras.new(name)
        cam.lens = number(params, "focal_length", 50.0, lo=1.0, hi=5000.0)
        obj = bpy.data.objects.new(name, cam)
    elif kind == "LIGHT":
        lt = enum_choice(params.get("light_type", "POINT"), LIGHT_TYPES, "'light_type'")
        light = bpy.data.lights.new(name, lt)
        light.energy = number(params, "energy", 1000.0 if lt in {"POINT", "SPOT", "AREA"} else 3.0, lo=0.0)
        c = color(params, "color", n=(3,))
        if c:
            light.color = c
        obj = bpy.data.objects.new(name, light)
    elif kind == "TEXT":
        font = bpy.data.curves.new(name, "FONT")
        font.body = string(params, "text", "Text")
        obj = bpy.data.objects.new(name, font)
    else:  # CURVE: a simple two-point bezier the caller can extend with run_python or modifiers
        curve = bpy.data.curves.new(name, "CURVE")
        curve.dimensions = "3D"
        spline = curve.splines.new("BEZIER")
        spline.bezier_points.add(1)
        for p, x in zip(spline.bezier_points, (-1.0, 1.0)):
            p.co = (x, 0, 0)
            p.handle_left_type = p.handle_right_type = "AUTO"
        obj = bpy.data.objects.new(name, curve)
    link_new_object(obj, params.get("collection"))
    _apply_placement(obj, params)
    return describe(obj)


@method("set_transform", mutating=True)
def set_transform(params, ctx):
    objs = get_objects(name_list(params, "names"))
    loc, rot, scale = vec3(params, "location"), vec3(params, "rotation"), vec3(params, "scale")
    if not (loc or rot or scale):
        raise OmnixError(INVALID_ARGUMENT, "give at least one of location, rotation (degrees), scale")
    relative = boolean(params, "relative", False)
    for o in objs:
        if loc:
            o.location = [a + b for a, b in zip(o.location, loc)] if relative else loc
        if rot:
            r = [math.radians(a) for a in rot]
            o.rotation_euler = [a + b for a, b in zip(o.rotation_euler, r)] if relative else r
        if scale:
            o.scale = [a * b for a, b in zip(o.scale, scale)] if relative else scale
    return {"objects": [describe(o) for o in objs]}


@method("delete_objects", mutating=True)
def delete_objects(params, ctx):
    objs = get_objects(name_list(params, "names"))
    names = [o.name for o in objs]
    for o in objs:
        bpy.data.objects.remove(o, do_unlink=True)
    purged = 0
    if boolean(params, "purge_orphans", False):
        before = _datablock_count()
        bpy.data.orphans_purge(do_recursive=True)
        purged = before - _datablock_count()
    return {"deleted": names, "purged_datablocks": purged}


def _datablock_count():
    return sum(
        len(c)
        for c in (bpy.data.meshes, bpy.data.materials, bpy.data.cameras, bpy.data.lights, bpy.data.curves, bpy.data.images)
    )


@method("duplicate_object", mutating=True)
def duplicate_object(params, ctx):
    src = get_object(params.get("name"))
    new = src.copy()
    if src.data is not None and not boolean(params, "linked", False):
        new.data = src.data.copy()
    new.name = string(params, "new_name", src.name)
    colls = src.users_collection or [bpy.context.scene.collection]
    for c in colls:
        c.objects.link(new)
    off = vec3(params, "offset")
    if off:
        new.location = [a + b for a, b in zip(new.location, off)]
    return describe(new)


@method("set_parent", mutating=True)
def set_parent(params, ctx):
    children = get_objects(name_list(params, "children"))
    pname = params.get("parent")
    keep = boolean(params, "keep_transform", True)
    parent = get_object(pname) if pname is not None else None
    for ch in children:
        if parent is not None:
            p = parent
            while p is not None:
                if p == ch:
                    raise OmnixError(INVALID_ARGUMENT, "parenting '%s' to '%s' would create a cycle" % (ch.name, parent.name))
                p = p.parent
    for ch in children:
        world = ch.matrix_world.copy()
        ch.parent = parent
        if parent is not None:
            ch.matrix_parent_inverse = parent.matrix_world.inverted() if keep else Matrix.Identity(4)
        elif keep:
            ch.matrix_world = world
    return {"parent": parent.name if parent else None, "children": [c.name for c in children]}


@method("select_objects")
def select_objects(params, ctx):
    mode = enum_choice(params.get("mode", "REPLACE"), {"REPLACE", "ADD", "REMOVE", "CLEAR"}, "'mode'")
    vl = bpy.context.view_layer
    objs = get_objects(name_list(params, "names")) if mode != "CLEAR" else []
    for o in objs:
        if o.name not in vl.objects:
            raise OmnixError(INVALID_ARGUMENT, "object '%s' is not in the active view layer" % o.name)
    if mode in {"REPLACE", "CLEAR"}:
        for o in vl.objects:
            o.select_set(False)
    for o in objs:
        o.select_set(mode != "REMOVE")
    if objs and mode in {"REPLACE", "ADD"} and boolean(params, "make_active", True):
        vl.objects.active = objs[0]
    return {
        "selected": sorted(o.name for o in vl.objects if o.select_get()),
        "active": vl.objects.active.name if vl.objects.active else None,
    }


@method("collection", mutating=True)
def collection(params, ctx):
    action = enum_choice(params.get("action"), {"CREATE", "MOVE", "LINK"}, "'action'")
    name = string(params, "name", required=True)
    if action == "CREATE":
        if name in bpy.data.collections:
            raise OmnixError(INVALID_ARGUMENT, "collection '%s' already exists" % name)
        parent = bpy.context.scene.collection
        if params.get("parent") is not None:
            parent = target_collection(params["parent"])
        coll = bpy.data.collections.new(name)
        parent.children.link(coll)
        return {"created": coll.name, "parent": parent.name}
    coll = target_collection(name)
    objs = get_objects(name_list(params, "objects"))
    for o in objs:
        if action == "MOVE":
            for c in list(o.users_collection):
                if c != coll:
                    c.objects.unlink(o)
        if o.name not in coll.objects:
            coll.objects.link(o)
    return {"collection": coll.name, "objects": [o.name for o in objs], "action": action.lower()}

