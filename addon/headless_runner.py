"""Run the add-on core inside ``blender --background`` (used by the tests and, later, ``serve-headless``).

    blender --background --factory-startup --python headless_runner.py -- --socket /tmp/x/b.sock [--allow-run-python]

Timers do not fire in --background, so this loops ``pump()`` directly. The ``headless.shutdown`` method is only
registered here, so a client can stop the process cleanly.
"""

import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from omnix_blender import core  # noqa: E402
from omnix_blender.dispatch import method  # noqa: E402


@method("headless.shutdown")
def _shutdown(params, ctx):
    ctx.shutdown_requested = True
    return {"stopping": True}


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    path = None
    allow = os.environ.get("OMNIX_BLENDER_ALLOW_RUN_PYTHON") == "1"
    folders = [f for f in os.environ.get("OMNIX_BLENDER_ALLOWED_FOLDERS", "").split(os.pathsep) if f]
    outside = os.environ.get("OMNIX_BLENDER_ALLOW_OUTSIDE") == "1"
    i = 0
    while i < len(argv):
        if argv[i] == "--socket":
            path = argv[i + 1]
            i += 1
        elif argv[i] == "--allow-run-python":
            allow = True
        elif argv[i] == "--allow-outside-files":
            outside = True
        elif argv[i] == "--allowed-folder":
            folders.append(argv[i + 1])
            i += 1
        i += 1
    core.start(
        path=path,
        allow_run_python=lambda: allow,
        headless=True,
        use_timer=False,
        allow_outside_files=lambda: outside,
        allowed_folders=lambda: folders,
    )
    print("OMNIX_READY %s" % core.socket_path(), flush=True)
    try:
        while not core.context().shutdown_requested:
            core.pump(block=0.05)
        core.pump()  # flush the shutdown response
        time.sleep(0.1)
    finally:
        core.stop()


main()
