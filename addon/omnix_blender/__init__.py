"""Omnix Blender: a local, same-user Unix-socket bridge that lets the omnix-blender MCP server drive Blender."""

bl_info = {
    "name": "Omnix Blender",
    "author": "Omnix",
    "version": (0, 1, 0),
    "blender": (4, 2, 0),
    "location": "3D Viewport > Sidebar > Omnix",
    "description": "Local MCP bridge for the omnix-blender server",
    "category": "Development",
}

import bpy

from . import core
from .protocol import OmnixError


def _prefs():
    try:
        return bpy.context.preferences.addons[__package__].preferences
    except (KeyError, AttributeError):
        return None


def _allow_run_python():
    p = _prefs()
    return bool(p and p.allow_run_python)


class OmnixPreferences(bpy.types.AddonPreferences):
    bl_idname = __package__

    allow_run_python: bpy.props.BoolProperty(
        name="Allow run_python",
        description="Let the MCP client execute arbitrary Python in this Blender session (remote code execution by design)",
        default=False,
    )
    auto_start: bpy.props.BoolProperty(
        name="Start on load",
        description="Start listening automatically when the add-on loads. Off by default: starting is an explicit click",
        default=False,
    )

    def draw(self, context):
        col = self.layout.column()
        col.prop(self, "allow_run_python")
        col.prop(self, "auto_start")
        col.label(text="Socket: %s" % core.socket_path())


class OMNIX_OT_start(bpy.types.Operator):
    bl_idname = "omnix.start"
    bl_label = "Start Omnix server"

    def execute(self, context):
        try:
            path = core.start(allow_run_python=_allow_run_python)
        except OmnixError as exc:
            self.report({"ERROR"}, exc.message)
            return {"CANCELLED"}
        self.report({"INFO"}, "Listening on %s" % path)
        return {"FINISHED"}


class OMNIX_OT_stop(bpy.types.Operator):
    bl_idname = "omnix.stop"
    bl_label = "Stop Omnix server"

    def execute(self, context):
        core.stop()
        return {"FINISHED"}


class OMNIX_PT_panel(bpy.types.Panel):
    bl_label = "Omnix"
    bl_idname = "OMNIX_PT_panel"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Omnix"

    def draw(self, context):
        layout = self.layout
        running = core.is_running()
        layout.label(text="Status: %s" % ("listening" if running else "stopped"), icon="CHECKMARK" if running else "X")
        if running:
            layout.operator("omnix.stop", icon="PAUSE")
        else:
            layout.operator("omnix.start", icon="PLAY")
        p = _prefs()
        if p:
            layout.prop(p, "allow_run_python")
        layout.label(text=core.socket_path())
        ctx = core.context()
        if ctx and ctx.last_request:
            import time

            name, ts = ctx.last_request
            layout.label(text="Last: %s (%ds ago)" % (name, time.time() - ts))


_classes = (OmnixPreferences, OMNIX_OT_start, OMNIX_OT_stop, OMNIX_PT_panel)


def _auto_start():
    p = _prefs()
    if p and p.auto_start and not core.is_running():
        try:
            core.start(allow_run_python=_allow_run_python)
        except OmnixError as exc:
            print("omnix_blender: auto-start failed: %s" % exc.message)
    return None


def register():
    for c in _classes:
        bpy.utils.register_class(c)
    # Preferences are not readable yet during register(); defer one tick.
    bpy.app.timers.register(_auto_start, first_interval=0.5, persistent=True)


def unregister():
    core.stop()
    if bpy.app.timers.is_registered(_auto_start):
        bpy.app.timers.unregister(_auto_start)
    for c in reversed(_classes):
        bpy.utils.unregister_class(c)
