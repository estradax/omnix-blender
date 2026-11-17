"""Enable -> start -> stop -> disable -> enable again, inside one Blender, against the installed extension.

Prints LIFECYCLE_OK on success; any failed check prints LIFECYCLE_FAIL <reason> and exits non-zero.
"""

import importlib
import json
import os
import socket
import stat
import sys
import threading

import addon_utils
import bpy

MOD = "bl_ext.user_default.omnix_blender"
SOCK = os.environ["OMNIX_BLENDER_SOCKET"]


def fail(msg):
    print("LIFECYCLE_FAIL " + msg, flush=True)
    os._exit(1)


def check(cond, msg):
    if not cond:
        fail(msg)


def omnix_threads():
    return [t for t in threading.enumerate() if t.name.startswith("omnix-")]


def client_calls(calls):
    """Run `calls` (list of (method, params)) from a client thread while this (main) thread pumps."""
    from importlib import import_module

    core = import_module(MOD + ".core")
    out = {}

    def run():
        try:
            s = socket.socket(socket.AF_UNIX)
            s.settimeout(10)
            s.connect(SOCK)
            f = s.makefile("rwb")
            res = []
            for i, (m, p) in enumerate([("hello", {})] + calls):
                f.write((json.dumps({"v": 1, "id": i, "method": m, "params": p}) + "\n").encode())
                f.flush()
                res.append(json.loads(f.readline()))
            out["res"] = res
            s.close()
        except Exception as exc:  # noqa: BLE001
            out["err"] = repr(exc)

    t = threading.Thread(target=run)
    t.start()
    while t.is_alive():
        core.pump(block=0.02)
    t.join()
    check("err" not in out, "client error: %s" % out.get("err"))
    return out["res"]


def enable():
    mod = addon_utils.enable(MOD, default_set=False, persistent=False)
    check(mod is not None, "enable failed")
    return mod


def cycle(label, allow_values):
    enable()
    prefs = bpy.context.preferences.addons[MOD].preferences
    check(hasattr(bpy.types, "OMNIX_OT_start"), "%s: operator not registered after enable" % label)
    check(not os.path.exists(SOCK), "%s: socket exists before start (auto-start must be off)" % label)
    prefs.allow_run_python = False
    check("FINISHED" in bpy.ops.omnix.start(), "%s: start failed" % label)
    st = os.stat(SOCK)
    check(stat.S_ISSOCK(st.st_mode), "%s: not a socket" % label)
    check(stat.S_IMODE(st.st_mode) == 0o600, "%s: socket mode %o" % (label, stat.S_IMODE(st.st_mode)))
    check((stat.S_IMODE(os.stat(os.path.dirname(SOCK)).st_mode) & 0o077) == 0, "%s: dir not private (%o)" % (label, stat.S_IMODE(os.stat(os.path.dirname(SOCK)).st_mode)))
    for allow in allow_values:
        prefs.allow_run_python = allow
        hello, rp = client_calls([("run_python", {"code": "result = 1 + 1", "undo": False})])
        check(hello["ok"] and hello["result"]["v"] == 1, "%s: bad hello %s" % (label, hello))
        if allow:
            check(rp["ok"] and rp["result"]["result"] == 2, "%s: run_python should work when allowed: %s" % (label, rp))
        else:
            check(not rp["ok"] and rp["error"]["code"] == "disabled", "%s: run_python must be disabled: %s" % (label, rp))
            check("Allow run_python" in rp["error"]["message"], "%s: message must say how to enable" % label)
    check("FINISHED" in bpy.ops.omnix.stop(), "%s: stop failed" % label)
    check(not os.path.exists(SOCK), "%s: socket file remained after stop" % label)
    check(not omnix_threads(), "%s: threads still alive after stop: %s" % (label, omnix_threads()))
    s = socket.socket(socket.AF_UNIX)
    try:
        s.connect(SOCK)
        fail("%s: connect succeeded after stop" % label)
    except OSError:
        pass


cycle("first", [False, True])
addon_utils.disable(MOD)
check(not hasattr(bpy.types, "OMNIX_OT_start"), "operator still registered after disable")
check(not os.path.exists(SOCK), "socket present after disable")
check(not omnix_threads(), "threads after disable")
# A running server must also be torn down by disable (not only by an explicit stop).
enable()
check("FINISHED" in bpy.ops.omnix.start(), "restart before disable failed")
addon_utils.disable(MOD)
check(not os.path.exists(SOCK), "socket still present after disable while running")
check(not omnix_threads(), "threads alive after disable while running")
cycle("second", [True, False])
print("LIFECYCLE_OK", flush=True)
