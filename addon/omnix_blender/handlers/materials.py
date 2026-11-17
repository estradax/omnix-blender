"""Materials and modifiers."""

import bpy

from ..dispatch import method
from ..protocol import INVALID_ARGUMENT, NOT_FOUND, OmnixError
from .common import color, get_object, int_param, number, string

MATERIAL_TYPES = {"MESH", "CURVE", "SURFACE", "FONT", "META", "VOLUME", "GREASEPENCIL", "POINTCLOUD"}


def _principled(mat):
    mat.use_nodes = True
    node = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if node is None:
        raise OmnixError(INVALID_ARGUMENT, "material '%s' has no Principled BSDF node to update" % mat.name)
    return node


@method("set_material", mutating=True)
def set_material(params, ctx):
    obj = get_object(params.get("object"))
    if obj.type not in MATERIAL_TYPES:
        raise OmnixError(INVALID_ARGUMENT, "objects of type %s cannot have materials" % obj.type)
    name = string(params, "name", obj.name + "_Mat")
    base = color(params, "base_color")
    emission = color(params, "emission", n=(3, 4))
    metallic = number(params, "metallic", lo=0.0, hi=1.0)
    roughness = number(params, "roughness", lo=0.0, hi=1.0)
    alpha = number(params, "alpha", lo=0.0, hi=1.0)
    strength = number(params, "emission_strength", lo=0.0)
    slot = int_param(params, "slot", 0, 0, 1000) if "slot" in params else None

    mat = bpy.data.materials.get(name)
    created = mat is None
    if created:
        mat = bpy.data.materials.new(name)
    node = _principled(mat)
    if base:
        node.inputs["Base Color"].default_value = (base + [1.0])[:4]
    if metallic is not None:
        node.inputs["Metallic"].default_value = metallic
    if roughness is not None:
        node.inputs["Roughness"].default_value = roughness
    if alpha is not None:
        node.inputs["Alpha"].default_value = alpha
        mat.surface_render_method = "BLENDED" if alpha < 1.0 else "DITHERED"
    if emission:
        node.inputs["Emission Color"].default_value = (emission + [1.0])[:4]
        if strength is None and any(emission[:3]):
            strength = 1.0
    if strength is not None:
        node.inputs["Emission Strength"].default_value = strength

    slots = obj.material_slots
    if slot is None:
        slot = next((i for i, s in enumerate(slots) if s.material == mat), None)
        if slot is None:
            slot = 0 if len(slots) else None
    if slot is None:
        obj.data.materials.append(mat)
        slot = len(obj.material_slots) - 1
    else:
        while len(obj.material_slots) <= slot:
            obj.data.materials.append(None)
        obj.material_slots[slot].material = mat
    inp = node.inputs
    return {
        "material": mat.name,
        "created": created,
        "object": obj.name,
        "slot": slot,
        "base_color": list(inp["Base Color"].default_value),
        "metallic": inp["Metallic"].default_value,
        "roughness": inp["Roughness"].default_value,
        "alpha": inp["Alpha"].default_value,
        "emission_strength": inp["Emission Strength"].default_value,
    }


def _modifier_types():
    return {e.identifier for e in bpy.types.Modifier.bl_rna.properties["type"].enum_items}


def _coerce(mod, key, value):
    props = mod.bl_rna.properties
    if key in {"type", "name", "rna_type"} or key not in props:
        valid = sorted(p.identifier for p in props if not p.is_readonly and p.identifier not in {"type", "name"})
        raise OmnixError(INVALID_ARGUMENT, "modifier %s has no settable property '%s'. Settable: %s" % (mod.type, key, ", ".join(valid)))
    prop = props[key]
    if prop.is_readonly:
        raise OmnixError(INVALID_ARGUMENT, "property '%s' is read-only" % key)
    t = prop.type
    try:
        if t == "POINTER":
            if value is None:
                return None
            if isinstance(value, str) and value in bpy.data.objects:
                return bpy.data.objects[value]
            if isinstance(value, str) and value in bpy.data.collections:
                return bpy.data.collections[value]
            raise OmnixError(NOT_FOUND, "'%s' expects the name of an existing object or collection, got %r" % (key, value))
        if t == "ENUM":
            valid = [e.identifier for e in prop.enum_items]
            if prop.is_enum_flag:
                return set(value) if isinstance(value, list) else {value}
            if value not in valid:
                raise OmnixError(INVALID_ARGUMENT, "'%s' must be one of: %s" % (key, ", ".join(valid)))
            return value
        if t == "BOOLEAN":
            if prop.array_length:
                return [bool(x) for x in value]
            if not isinstance(value, bool):
                raise OmnixError(INVALID_ARGUMENT, "'%s' must be true or false" % key)
            return value
        if t == "INT":
            if prop.array_length:
                return [int(x) for x in value]
            if isinstance(value, bool) or not isinstance(value, (int, float)) or int(value) != value:
                raise OmnixError(INVALID_ARGUMENT, "'%s' must be an integer" % key)
            return int(value)
        if t == "FLOAT":
            if prop.array_length:
                return [float(x) for x in value]
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                raise OmnixError(INVALID_ARGUMENT, "'%s' must be a number" % key)
            return float(value)
        if t == "STRING":
            if not isinstance(value, str):
                raise OmnixError(INVALID_ARGUMENT, "'%s' must be a string" % key)
            return value
    except (TypeError, ValueError) as exc:
        raise OmnixError(INVALID_ARGUMENT, "bad value for '%s': %s" % (key, exc))
    raise OmnixError(INVALID_ARGUMENT, "property '%s' has unsupported type %s" % (key, t))


def _set_props(mod, props):
    if props is None:
        return
    if not isinstance(props, dict):
        raise OmnixError(INVALID_ARGUMENT, "'properties' must be an object")
    for key, value in props.items():
        coerced = _coerce(mod, key, value)
        try:
            setattr(mod, key, coerced)
        except (TypeError, ValueError, AttributeError, RuntimeError) as exc:
            raise OmnixError(INVALID_ARGUMENT, "cannot set '%s': %s" % (key, exc))


def _get_modifier(obj, name):
    mod = obj.modifiers.get(name) if isinstance(name, str) else None
    if mod is None:
        have = ", ".join(m.name for m in obj.modifiers) or "none"
        raise OmnixError(NOT_FOUND, "object '%s' has no modifier '%s' (it has: %s)" % (obj.name, name, have))
    return mod


@method("add_modifier", mutating=True)
def add_modifier(params, ctx):
    obj = get_object(params.get("object"))
    mtype = params.get("type")
    types = _modifier_types()
    if not isinstance(mtype, str) or mtype.upper() not in types:
        raise OmnixError(INVALID_ARGUMENT, "'type' must be a modifier type such as SUBSURF, BEVEL, MIRROR, ARRAY, BOOLEAN, SOLIDIFY, DECIMATE. All: %s" % ", ".join(sorted(types)))
    mtype = mtype.upper()
    name = string(params, "name", mtype.title())
    try:
        mod = obj.modifiers.new(name, mtype)
    except (TypeError, RuntimeError) as exc:
        raise OmnixError(INVALID_ARGUMENT, "cannot add %s to a %s object: %s" % (mtype, obj.type, exc))
    if mod is None:
        raise OmnixError(INVALID_ARGUMENT, "cannot add %s to a %s object" % (mtype, obj.type))
    try:
        _set_props(mod, params.get("properties"))
    except OmnixError:
        obj.modifiers.remove(mod)  # leave nothing half-configured
        raise
    return {"object": obj.name, "modifier": mod.name, "type": mod.type, "modifiers": [m.name for m in obj.modifiers]}


@method("apply_modifier", mutating=True)
def apply_modifier(params, ctx):
    obj = get_object(params.get("object"))
    mod = _get_modifier(obj, params.get("name"))
    if obj.data is not None and obj.data.users > 1:
        raise OmnixError(INVALID_ARGUMENT, "'%s' shares its data with other objects; make it single-user first" % obj.name)
    name = mod.name
    try:
        with bpy.context.temp_override(object=obj, active_object=obj, selected_objects=[obj]):
            bpy.ops.object.modifier_apply(modifier=name)
    except RuntimeError as exc:
        raise OmnixError(INVALID_ARGUMENT, "cannot apply modifier '%s': %s" % (name, str(exc).strip()))
    return {"object": obj.name, "applied": name, "modifiers": [m.name for m in obj.modifiers]}


@method("remove_modifier", mutating=True)
def remove_modifier(params, ctx):
    obj = get_object(params.get("object"))
    mod = _get_modifier(obj, params.get("name"))
    name = mod.name
    obj.modifiers.remove(mod)
    return {"object": obj.name, "removed": name, "modifiers": [m.name for m in obj.modifiers]}
