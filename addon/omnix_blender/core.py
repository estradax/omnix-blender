"""Process-wide add-on state: start/stop the server and drive ``pump()``."""

import sys
import threading

from . import handlers  # noqa: F401  (registers methods)
from . import paths
from .dispatch import Context, Dispatcher, METHODS
from .protocol import PROTOCOL_VERSION
from .server import Server

ADDON_VERSION = "0.1.0"
PUMP_INTERVAL = 0.02

_lock = threading.Lock()
_server = None
_dispatcher = None
_ctx = None
_timer_registered = False


def is_running():
    return _server is not None


def socket_path():
    return _server.path if _server else paths.default_socket_path()


def context():
    return _ctx


def start(
    path=None,
    allow_run_python=lambda: False,
    headless=False,
    use_timer=True,
    allow_outside_files=lambda: False,
    allowed_folders=lambda: [],
):
    """Start listening. Raises OmnixError if the socket path is unusable."""
    import bpy

    global _server, _dispatcher, _ctx, _timer_registered
    with _lock:
        if _server is not None:
            return _server.path
        path = path or paths.default_socket_path()
        _ctx = Context(
            allow_run_python=allow_run_python,
            headless=headless,
            allow_outside_files=allow_outside_files,
            allowed_folders=allowed_folders,
        )
        _dispatcher = Dispatcher(_ctx)
        info = {
            "v": PROTOCOL_VERSION,
            "blender": bpy.app.version_string,
            "addon": ADDON_VERSION,
            "features": sorted(METHODS),
            "params": {name: sorted(m.keys) for name, m in sorted(METHODS.items())},
            "headless": headless,
        }
        _initial_undo_step()
        server = Server(path, _dispatcher, info)
        server.start()
        _server = server
        if use_timer:
            bpy.app.timers.register(_timer, first_interval=PUMP_INTERVAL, persistent=True)
            _timer_registered = True
        return path


def pump(block=0.0):
    """The single entry point that executes queued requests; main thread only."""
    d = _dispatcher
    return d.pump(block=block) if d else 0


def _timer():
    if _server is None:
        return None
    try:
        pump()
    except Exception:  # noqa: BLE001 - never let the timer die
        import traceback

        traceback.print_exc(file=sys.stderr)
    return PUMP_INTERVAL


def stop():
    global _server, _dispatcher, _ctx, _timer_registered
    with _lock:
        server, _server = _server, None
        if _timer_registered:
            import bpy

            try:
                bpy.app.timers.unregister(_timer)
            except ValueError:
                pass
            _timer_registered = False
        if server is not None:
            server.stop()
        _dispatcher = None


def _initial_undo_step():
    """Make sure the state at start is an undo target (headless sessions have no 'original' step)."""
    import bpy

    try:
        bpy.ops.ed.undo_push(message="Omnix: start")
    except Exception:  # noqa: BLE001
        pass
