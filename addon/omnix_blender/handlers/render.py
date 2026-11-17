"""Render settings, rendering to a private temp file, and viewport screenshots."""

import os
import tempfile

import bpy

from ..dispatch import method
from ..protocol import INVALID_ARGUMENT, OmnixError, UNSUPPORTED
from .common import enum_choice, get_object, int_param, string

ENGINES = {"BLENDER_EEVEE", "CYCLES", "BLENDER_WORKBENCH"}
RENDER_DIR_PREFIX = "omnix-render-"
_render_dir = None
_counter = 0


def _private_dir():
    """A 0700 temp dir (mkdtemp) owned by this process, so renders are not readable by other users."""
    global _render_dir
    if _render_dir is None or not os.path.isdir(_render_dir):
        _render_dir = tempfile.mkdtemp(prefix=RENDER_DIR_PREFIX)
    return _render_dir


def _next_path(stem):
    global _counter
    _counter += 1
    d = _private_dir()
    old = sorted(f for f in os.listdir(d) if f.endswith(".png"))
    for f in old[:-19]:  # keep only the latest ~20 images
        try:
            os.unlink(os.path.join(d, f))
        except OSError:
            pass
    return os.path.join(d, "%s_%04d.png" % (stem, _counter))


def _settings(sc):
    r = sc.render
    samples = sc.cycles.samples if (r.engine == "CYCLES" and hasattr(sc, "cycles")) else getattr(sc.eevee, "taa_render_samples", None)
    return {
        "engine": r.engine,
        "resolution_x": r.resolution_x,
        "resolution_y": r.resolution_y,
        "resolution_percentage": r.resolution_percentage,
        "samples": samples,
        "output_format": r.image_settings.file_format,
        "output_path": r.filepath,
    }


def _set_samples(sc, n):
    if sc.render.engine == "CYCLES":
        sc.cycles.samples = n
    elif sc.render.engine == "BLENDER_EEVEE":
        sc.eevee.taa_render_samples = n


@method("set_render_settings", mutating=True)
def set_render_settings(params, ctx):
    sc = bpy.context.scene
    r = sc.render
    if "engine" in params:
        eng = params["engine"]
        eng = "BLENDER_EEVEE" if isinstance(eng, str) and eng.upper() in {"EEVEE", "EEVEE_NEXT"} else eng
        r.engine = enum_choice(eng, ENGINES, "'engine'")
    for key, attr in (("resolution_x", "resolution_x"), ("resolution_y", "resolution_y")):
        if key in params:
            setattr(r, attr, int_param(params, key, 0, 1, 16384))
    if "resolution_percentage" in params:
        r.resolution_percentage = int_param(params, "resolution_percentage", 100, 1, 1000)
    if "samples" in params:
        _set_samples(sc, int_param(params, "samples", 1, 1, 100000))
    if "output_format" in params:
        valid = {e.identifier for e in r.image_settings.bl_rna.properties["file_format"].enum_items}
        r.image_settings.file_format = enum_choice(params["output_format"], valid, "'output_format'")
    if "output_path" in params:
        r.filepath = string(params, "output_path", required=True)
    return _settings(sc)


@method("render_image", mutating=False)
def render_image(params, ctx):
    sc = bpy.context.scene
    r = sc.render
    cam_name = params.get("camera")
    cam = get_object(cam_name) if cam_name is not None else sc.camera
    if cam is None or cam.type != "CAMERA":
        raise OmnixError(INVALID_ARGUMENT, "no camera to render from: the scene has no active camera. Create one with create_object type=CAMERA, or pass 'camera'.")
    samples = int_param(params, "samples", 0, 0, 100000) if "samples" in params else None
    pct = int_param(params, "resolution_percentage", 0, 1, 1000) if "resolution_percentage" in params else None
    path = _next_path("render")
    saved = (sc.camera, r.filepath, r.image_settings.file_format, r.image_settings.color_mode, r.resolution_percentage)
    saved_samples = _settings(sc)["samples"]
    try:
        sc.camera = cam
        r.filepath = path
        r.image_settings.file_format = "PNG"
        r.image_settings.color_mode = "RGBA" if r.film_transparent else "RGB"
        if pct:
            r.resolution_percentage = pct
        if samples:
            _set_samples(sc, samples)
        bpy.ops.render.render(write_still=True)
    finally:
        sc.camera, r.filepath, r.image_settings.file_format, r.image_settings.color_mode, r.resolution_percentage = saved
        if saved_samples is not None:
            _set_samples(sc, saved_samples)
    if not os.path.isfile(path):
        raise OmnixError("blender_error", "render finished but no image was written")
    return {
        "path": path,
        "bytes": os.path.getsize(path),
        "width": int(r.resolution_x * (pct or r.resolution_percentage) / 100),
        "height": int(r.resolution_y * (pct or r.resolution_percentage) / 100),
        "camera": cam.name,
        "engine": r.engine,
    }


@method("viewport_screenshot")
def viewport_screenshot(params, ctx):
    if bpy.app.background:
        raise OmnixError(UNSUPPORTED, "viewport_screenshot needs the Blender GUI; this Blender runs headless (--background)")
    wm = bpy.context.window_manager
    for win in wm.windows:
        for area in win.screen.areas:
            if area.type == "VIEW_3D":
                region = next((rg for rg in area.regions if rg.type == "WINDOW"), None)
                if region is None:
                    continue
                path = _next_path("viewport")
                with bpy.context.temp_override(window=win, area=area, region=region):
                    bpy.ops.screen.screenshot_area(filepath=path)
                if not os.path.isfile(path):
                    raise OmnixError("blender_error", "screenshot was not written")
                return {"path": path, "bytes": os.path.getsize(path)}
    raise OmnixError(UNSUPPORTED, "no 3D viewport is open in this Blender window")


