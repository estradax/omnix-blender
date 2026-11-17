"""Run inside Blender (not --factory-startup) with the *installed* extension, in an isolated resource dir.

Env: OMNIX_BLENDER_SOCKET, OMNIX_TEST_ALLOW (0/1: the 'Allow run_python' preference), OMNIX_TEST_STOP (flag file).
Starts the server through the add-on's own operator, then serves until the flag file appears.
Background: loops core.pump() (timers do not fire headless). GUI: the add-on's own timer pumps.
"""

import importlib
import os
import sys

import addon_utils
import bpy

MOD = "bl_ext.user_default.omnix_blender"
stop_flag = os.environ["OMNIX_TEST_STOP"]
allow = os.environ.get("OMNIX_TEST_ALLOW") == "1"

mod = addon_utils.enable(MOD, default_set=False, persistent=False) or sys.modules.get(MOD)
if mod is None:
    print("SESSION_FAIL extension not enabled", flush=True)
    bpy.ops.wm.quit_blender()
core = importlib.import_module(MOD + ".core")
prefs = bpy.context.preferences.addons[MOD].preferences
prefs.allow_run_python = allow


def start():
    res = bpy.ops.omnix.start()
    print("SESSION_READY" if "FINISHED" in res and core.is_running() else "SESSION_FAIL start %s" % res, flush=True)


if bpy.app.background:
    start()
    while not os.path.exists(stop_flag):
        core.pump(block=0.05)
    bpy.ops.omnix.stop()
else:

    def first():
        start()
        bpy.app.timers.register(watch, first_interval=0.5, persistent=True)

    def watch():
        if os.path.exists(stop_flag):
            bpy.ops.omnix.stop()
            bpy.ops.wm.quit_blender()
            return None
        return 0.3

    bpy.app.timers.register(first, first_interval=0.5, persistent=True)
