"""run_python: arbitrary code execution, gated by the add-on preference (checked on every call)."""

import contextlib
import io
import traceback

import bpy

from ..dispatch import method
from ..protocol import DISABLED, INVALID_ARGUMENT, OmnixError

DISABLED_MESSAGE = (
    "run_python is disabled. To enable it: in Blender open Edit > Preferences > Add-ons > Omnix Blender and tick "
    "'Allow run_python' (or tick it in the 3D viewport sidebar: N panel > Omnix). It lets the MCP client run any "
    "Python in this Blender session, so only enable it if you trust the client."
)


@method("run_python", mutating=True, keys=("code", "timeout_s", "undo", ))
def run_python(params, ctx):
    if not ctx.allow_run_python():
        raise OmnixError(DISABLED, DISABLED_MESSAGE)
    code = params.get("code")
    if not isinstance(code, str) or not code.strip():
        raise OmnixError(INVALID_ARGUMENT, "'code' must be a non-empty string")
    if params.get("undo") is False:
        ctx.skip_undo = True
    import bmesh
    import mathutils

    ns = {"bpy": bpy, "bmesh": bmesh, "mathutils": mathutils, "__name__": "__omnix_run_python__"}
    out = io.StringIO()
    error = None
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
        try:
            exec(compile(code, "<run_python>", "exec"), ns)
        except BaseException as exc:  # noqa: BLE001 - report SystemExit/KeyboardInterrupt from user code too
            tb = traceback.format_exc().strip().splitlines()
            error = {"type": type(exc).__name__, "message": str(exc), "traceback": "\n".join(tb[-8:])}
    result = {"stdout": out.getvalue()[-65536:], "result": ns.get("result")}
    if error:
        result["error"] = error
    return result
