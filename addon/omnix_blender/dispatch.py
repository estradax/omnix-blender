"""Main-thread dispatcher.

Socket threads call :meth:`Dispatcher.submit` and wait; Blender's main thread calls :meth:`Dispatcher.pump`
(from a ``bpy.app.timers`` callback in the GUI, or a plain loop when headless). ``bpy`` is only touched from
``pump()`` and the handlers it runs.
"""

import queue
import threading
import time
import traceback

from . import protocol
from .protocol import OmnixError

# name -> Method
METHODS = {}


class Method:
    def __init__(self, name, fn, mutating, undo_label, keys):
        self.keys = keys  # accepted `params` keys (frozenset); anything else is rejected
        self.name = name
        self.fn = fn
        self.mutating = mutating
        self.undo_label = undo_label


def method(name, mutating=False, undo_label=None, keys=()):
    """Register a handler ``fn(params, ctx) -> json-able``. Mutating handlers get one undo step."""

    def deco(fn):
        METHODS[name] = Method(name, fn, mutating, undo_label or name, frozenset(keys))
        return fn

    return deco


class Pending:
    def __init__(self, req):
        self.req = req
        self.event = threading.Event()
        self.response = None
        self.cancelled = False
        self.started = False


class Context:
    """Per-process settings handlers can consult. ``allow_run_python`` is read on the main thread."""

    def __init__(
        self,
        allow_run_python=lambda: False,
        headless=False,
        allow_outside_files=lambda: False,
        allowed_folders=lambda: [],
    ):
        self.allow_run_python = allow_run_python
        self.allow_outside_files = allow_outside_files
        self.allowed_folders = allowed_folders
        self.headless = headless
        self.shutdown_requested = False
        self.last_request = None  # (method, time) for the UI
        self.dirty = False  # unsaved changes made through this add-on
        self.skip_undo = False  # a mutating handler may ask to skip its undo step (run_python undo=false)


class Dispatcher:
    def __init__(self, ctx):
        self.ctx = ctx
        self._queue = queue.Queue()
        self._lock = threading.Lock()
        self._outstanding = 0

    # ---- called from socket threads -------------------------------------------------
    def submit(self, req):
        """Queue a request. Returns a Pending, or None when another request is still outstanding (busy)."""
        with self._lock:
            if self._outstanding:
                return None
            self._outstanding += 1
        pending = Pending(req)
        self._queue.put(pending)
        return pending

    def wait(self, pending, timeout_s, stop=None):
        """Wait for the result; on timeout (or when ``stop`` is set) cancel it if it has not started."""
        req_id = pending.req.get("id")
        deadline = time.monotonic() + timeout_s
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            if pending.event.wait(min(0.1, remaining)):
                return pending.response
            if stop is not None and stop.is_set():
                pending.cancelled = True
                return protocol.err(req_id, protocol.DISABLED, "the Omnix server in Blender is stopping")
        pending.cancelled = True
        if pending.started:
            msg = (
                "request '%s' exceeded %.0fs and is still running in Blender (it cannot be interrupted); "
                "further requests return 'busy' until it finishes" % (pending.req["method"], timeout_s)
            )
        else:
            msg = "request '%s' was not picked up within %.0fs (is Blender busy or minimised?)" % (
                pending.req["method"],
                timeout_s,
            )
        return protocol.err(req_id, protocol.TIMEOUT, msg)

    # ---- called from the main thread ------------------------------------------------
    def pump(self, max_items=8, block=0.0):
        """Run queued requests on the calling (main) thread. Returns the number executed."""
        done = 0
        while done < max_items:
            try:
                pending = self._queue.get(timeout=block) if (block and done == 0) else self._queue.get_nowait()
            except queue.Empty:
                break
            try:
                if pending.cancelled:
                    continue
                pending.started = True
                pending.response = self._execute(pending.req)
                pending.event.set()
                done += 1
            finally:
                with self._lock:
                    self._outstanding -= 1
        return done

    def _execute(self, req):
        req_id = req.get("id")
        name = req["method"]
        m = METHODS.get(name)
        if m is None:
            return protocol.err(req_id, protocol.UNSUPPORTED, "unknown method '%s'" % name)
        extra = sorted(set(req["params"]) - m.keys)
        if extra:
            return protocol.err(
                req_id,
                protocol.INVALID_ARGUMENT,
                "unknown parameter(s) for %s: %s (accepted: %s)" % (name, ", ".join(map(str, extra)), ", ".join(sorted(m.keys)) or "none"),
            )
        self.ctx.last_request = (name, time.time())
        try:
            self.ctx.skip_undo = False
            result = m.fn(req["params"], self.ctx)
            if m.mutating and not self.ctx.skip_undo:
                self.ctx.dirty = True
                _undo_push(m.undo_label)
            return protocol.ok(req_id, result)
        except OmnixError as exc:
            return protocol.err(req_id, exc.code, exc.message)
        except Exception as exc:  # noqa: BLE001 - every handler failure must become a response
            tb = traceback.format_exc().strip().splitlines()
            short = "\n".join(tb[-6:])
            return protocol.err(req_id, protocol.BLENDER_ERROR, "%s: %s\n%s" % (type(exc).__name__, exc, short))


def _undo_push(label):
    import bpy

    try:
        bpy.ops.ed.undo_push(message="Omnix: %s" % label)
    except Exception:  # noqa: BLE001 - an undo push failing must not fail the request
        pass
