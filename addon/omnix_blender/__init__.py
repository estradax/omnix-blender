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


def _allow_outside_files():
    p = _prefs()
    return bool(p and p.allow_outside_files)


def _allowed_folders():
    p = _prefs()
    return [f.path for f in p.allowed_folders] if p else []


def _start():
    return core.start(
        allow_run_python=_allow_run_python,
        allow_outside_files=_allow_outside_files,
        allowed_folders=_allowed_folders,
    )


class OmnixFolder(bpy.types.PropertyGroup):
    path: bpy.props.StringProperty(name="Folder", subtype="DIR_PATH")


class OMNIX_OT_add_folder(bpy.types.Operator):
    bl_idname = "omnix.add_folder"
    bl_label = "Add allowed folder"

    directory: bpy.props.StringProperty(subtype="DIR_PATH")

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        p = _prefs()
        if p and self.directory:
            p.allowed_folders.add().path = self.directory
        return {"FINISHED"}


class OMNIX_OT_remove_folder(bpy.types.Operator):
    bl_idname = "omnix.remove_folder"
    bl_label = "Remove allowed folder"

    index: bpy.props.IntProperty()

    def execute(self, context):
        p = _prefs()
        if p and 0 <= self.index < len(p.allowed_folders):
            p.allowed_folders.remove(self.index)
        return {"FINISHED"}


class OmnixPreferences(bpy.types.AddonPreferences):
    bl_idname = __package__

    allow_run_python: bpy.props.BoolProperty(
        name="Allow run_python",
        description="Let the MCP client execute arbitrary Python in this Blender session (remote code execution by design)",
        default=False,
    )
    allow_outside_files: bpy.props.BoolProperty(
        name="Allow file access outside allowed folders",
        description="Let the MCP client read/write files anywhere your user can (hidden folders and protected "
        "locations such as ~/.ssh stay blocked). Off by default: only the .blend's folder and 'Allowed folders' are reachable",
        default=False,
    )
    allowed_folders: bpy.props.CollectionProperty(type=OmnixFolder)
    auto_start: bpy.props.BoolProperty(
        name="Start on load",
        description="Start listening automatically when the add-on loads. Off by default: starting is an explicit click",
        default=False,
    )

    def draw(self, context):
        col = self.layout.column()
        col.prop(self, "allow_run_python")
        col.prop(self, "allow_outside_files")
        col.label(text="Allowed folders (besides the saved .blend's folder):")
        for i, f in enumerate(self.allowed_folders):
            row = col.row()
            row.prop(f, "path", text="")
            row.operator("omnix.remove_folder", text="", icon="X").index = i
        col.operator("omnix.add_folder", icon="ADD")
        col.prop(self, "auto_start")
        col.label(text="Socket: %s" % core.socket_path())


class OMNIX_OT_start(bpy.types.Operator):
    bl_idname = "omnix.start"
    bl_label = "Start Omnix server"

    def execute(self, context):
        try:
            path = _start()
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
            layout.prop(p, "allow_outside_files")
        layout.label(text=core.socket_path())
        ctx = core.context()
        if ctx and ctx.last_request:
            import time

            name, ts = ctx.last_request
            layout.label(text="Last: %s (%ds ago)" % (name, time.time() - ts))


_classes = (OmnixFolder, OMNIX_OT_add_folder, OMNIX_OT_remove_folder, OmnixPreferences, OMNIX_OT_start, OMNIX_OT_stop, OMNIX_PT_panel)


def _auto_start():
    p = _prefs()
    if p and p.auto_start and not core.is_running():
        try:
            _start()
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
