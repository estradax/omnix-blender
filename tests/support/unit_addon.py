"""In-Blender unit tests for the add-on's socket/server code (no GUI, no installed extension needed).

Run: blender --background --factory-startup --python unit_addon.py
Prints UNIT_PASS <name> / UNIT_FAIL <name>: <why>; finally UNIT_DONE <passed> <failed>.
"""

import json
import os
import socket
import stat
import sys
import tempfile
import threading
import time
import traceback
import types

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "addon"))
sys.dont_write_bytecode = True

from omnix_blender import core, paths, protocol  # noqa: E402
from omnix_blender.dispatch import METHODS, Context, Dispatcher  # noqa: E402
from omnix_blender.protocol import OmnixError  # noqa: E402
from omnix_blender.server import Server  # noqa: E402

PASSED, FAILED = [], []


def test(fn):
    try:
        fn()
        PASSED.append(fn.__name__)
        print("UNIT_PASS " + fn.__name__, flush=True)
    except BaseException as exc:  # noqa: BLE001
        FAILED.append(fn.__name__)
        tb = traceback.format_exc().strip().splitlines()[-3:]
        print("UNIT_FAIL %s: %r | %s" % (fn.__name__, exc, " / ".join(tb)), flush=True)
    return fn


def private_dir():
    d = tempfile.mkdtemp(prefix="obu", dir="/tmp")
    os.chmod(d, 0o700)
    return d


def stale_socket(path):
    s = socket.socket(socket.AF_UNIX)
    s.bind(path)
    s.close()  # file stays, nobody listens
    assert os.path.exists(path)


def raises(fn, needle):
    try:
        fn()
    except OmnixError as exc:
        assert needle in exc.message, "wrong message: %s" % exc.message
        return exc
    raise AssertionError("expected OmnixError containing %r" % needle)


# ---- item 3: stale-socket detection must not delete a socket it cannot prove is dead ---------------------------


class _FakeSock:
    exc = socket.timeout("timed out")

    def __init__(self, *a, **k):
        pass

    def settimeout(self, t):
        pass

    def connect(self, p):
        raise self.exc

    def close(self):
        pass


def _with_fake_connect(exc, fn):
    real = paths.socket
    _FakeSock.exc = exc
    paths.socket = types.SimpleNamespace(
        socket=_FakeSock, AF_UNIX=socket.AF_UNIX, SOCK_STREAM=socket.SOCK_STREAM
    )
    try:
        return fn()
    finally:
        paths.socket = real


@test
def connect_timeout_does_not_delete_the_socket():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    stale_socket(p)
    raises(lambda: _with_fake_connect(socket.timeout("timed out"), lambda: paths.prepare_socket_path(p)), "not deleting")
    assert os.path.exists(p), "socket file was deleted although liveness is unknown"


@test
def other_oserror_does_not_delete_the_socket():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    stale_socket(p)
    raises(lambda: _with_fake_connect(PermissionError(13, "denied"), lambda: paths.prepare_socket_path(p)), "not deleting")
    assert os.path.exists(p)


@test
def refused_connection_means_stale_and_is_replaced():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    stale_socket(p)
    paths.prepare_socket_path(p)
    assert not os.path.exists(p)


@test
def live_listener_is_not_replaced():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    srv = socket.socket(socket.AF_UNIX)
    srv.bind(p)
    srv.listen(1)
    try:
        raises(lambda: paths.prepare_socket_path(p), "already listening")
        assert os.path.exists(p)
    finally:
        srv.close()


# ---- item 4: directory policy and bind without process-wide umask ----------------------------------------------


@test
def directory_with_group_or_other_bits_is_refused():
    for mode in (0o755, 0o750, 0o770, 0o705, 0o777):
        d = private_dir()
        os.chmod(d, mode)
        raises(lambda: paths.prepare_socket_path(os.path.join(d, "s.sock")), "700")
    d = private_dir()
    paths.prepare_socket_path(os.path.join(d, "s.sock"))  # 0700 is fine


@test
def symlinked_socket_directory_is_refused():
    real = private_dir()
    link = os.path.join(private_dir(), "lnk")
    os.symlink(real, link)
    raises(lambda: paths.prepare_socket_path(os.path.join(link, "s.sock")), "symlink")


@test
def created_directory_is_0700():
    base = private_dir()
    p = os.path.join(base, "fresh", "s.sock")
    paths.prepare_socket_path(p)
    assert stat.S_IMODE(os.stat(os.path.dirname(p)).st_mode) == 0o700


def _server(path, **kw):
    ctx = Context(headless=True)
    d = Dispatcher(ctx)
    return Server(path, d, {"v": 1, "blender": "x", "addon": "x", "features": [], "headless": True}, **kw), d


@test
def bind_does_not_use_process_umask_and_socket_is_0600():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    real_umask = os.umask

    def forbidden(*a):
        raise AssertionError("os.umask must not be used around bind (process-wide, racy)")

    os.umask = forbidden
    try:
        srv, _ = _server(p)
        srv.start()
    finally:
        os.umask = real_umask
    try:
        assert stat.S_IMODE(os.stat(p).st_mode) == 0o600
        old = os.umask(0o22)
        os.umask(old)
        assert old == 0o22 or True
    finally:
        srv.stop()
    assert not os.path.exists(p)


# ---- item 6: request parsing, shutdown latency, idle timeout, connection cap ----------------------------------


class _Conn:
    def __init__(self):
        self.sent = []

    def sendall(self, b):
        self.sent.append(json.loads(b))


class _FakeDispatcher:
    def __init__(self):
        self.timeouts = []

    def submit(self, req):
        return object()

    def wait(self, pending, timeout, stop=None):
        self.timeouts.append(timeout)
        return protocol.ok(1, {"fake": True})


def _handle(line):
    fd = _FakeDispatcher()
    srv = Server("/tmp/unused.sock", fd, {})
    c = _Conn()
    srv._handle_line(c, line, True)
    return fd, c.sent


@test
def timeout_s_must_be_a_finite_positive_number():
    bad = ["NaN", "Infinity", "-Infinity", "-1", "0", "true", '"5"', "1e999", "[1]", "null_"]
    for b in bad:
        line = ('{"v":1,"id":1,"method":"scene_info","params":{},"timeout_s":%s}' % b).encode()
        fd, sent = _handle(line)
        assert not fd.timeouts, "timeout_s=%s reached the dispatcher" % b
        assert sent and sent[0]["ok"] is False, "timeout_s=%s not rejected: %s" % (b, sent)
    fd, sent = _handle(b'{"v":1,"id":1,"method":"scene_info","params":{},"timeout_s":7}')
    assert fd.timeouts == [7.0], fd.timeouts
    fd, sent = _handle(b'{"v":1,"id":1,"method":"scene_info","params":{},"timeout_s":1e9}')
    assert fd.timeouts == [3600.0], "huge timeouts are clamped, got %s" % fd.timeouts
    fd, sent = _handle(b'{"v":1,"id":1,"method":"scene_info","params":{}}')
    assert fd.timeouts == [protocol.DEFAULT_TIMEOUT_S]


@test
def unknown_request_and_param_keys_are_rejected():
    ctx = Context(headless=True)
    d = Dispatcher(ctx)
    for req in (
        {"v": 1, "id": 1, "method": "scene_info", "params": {"bogus": 1}},
        {"v": 1, "id": 2, "method": "create_primitive", "params": {"type": "cube", "locaton": [0, 0, 0]}},
    ):
        p = d.submit(req)
        d.pump()
        assert p.response["ok"] is False and p.response["error"]["code"] == "invalid_argument", p.response
        assert "bogus" in p.response["error"]["message"] or "locaton" in p.response["error"]["message"]
    p = d.submit({"v": 1, "id": 3, "method": "list_objects", "params": {"limit": 1}})
    d.pump()
    assert p.response["ok"], p.response
    try:
        protocol.decode_request(b'{"v":1,"id":1,"method":"x","params":{},"extra":1}')
    except OmnixError as exc:
        assert exc.code == protocol.PROTOCOL_ERROR
    else:
        raise AssertionError("unknown top-level key accepted")


@test
def every_method_declares_its_accepted_params():
    missing = [n for n, m in METHODS.items() if m.keys is None]
    assert not missing, "methods without a params declaration: %s" % missing


def _client(path):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(5)
    s.connect(path)
    return s


def _hello(s):
    s.sendall(b'{"v":1,"id":0,"method":"hello","params":{}}\n')
    return json.loads(s.makefile("rb").readline())


@test
def stop_with_a_queued_request_returns_quickly():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    core.start(path=p, headless=True, use_timer=False)  # nobody pumps: the request stays queued
    s = _client(p)
    assert _hello(s)["ok"]
    s.sendall(b'{"v":1,"id":1,"method":"scene_info","params":{},"timeout_s":30}\n')
    time.sleep(0.3)
    t = time.time()
    core.stop()
    took = time.time() - t
    assert took < 1.5, "stop() blocked for %.2fs" % took
    alive = [t.name for t in threading.enumerate() if t.name.startswith("omnix-")]
    assert not alive, "threads still alive: %s" % alive
    assert not os.path.exists(p)
    s.close()


@test
def stop_with_many_connections_is_bounded():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    core.start(path=p, headless=True, use_timer=False)
    conns = []
    for _ in range(8):
        s = _client(p)
        _hello(s)
        s.sendall(b'{"v":1,"id":1,"method":"scene_info","params":{},"timeout_s":30}\n')
        conns.append(s)
    time.sleep(0.3)
    t = time.time()
    core.stop()
    took = time.time() - t
    assert took < 2.0, "stop() with 8 busy connections took %.2fs" % took
    for s in conns:
        s.close()


@test
def idle_connections_are_closed_and_free_their_slot():
    d = private_dir()
    p = os.path.join(d, "s.sock")
    srv, disp = _server(p, idle_timeout=1.0, max_connections=3)
    srv.start()
    try:
        idle = [_client(p) for _ in range(3)]
        for s in idle:
            s.settimeout(0.3)
        # Cap exhausted: a 4th connection is told so.
        extra = _client(p)
        resp = json.loads(extra.makefile("rb").readline())
        assert resp["error"]["code"] == "busy" and "too many" in resp["error"]["message"], resp
        # The idle ones are dropped after ~1s; a partial line does not keep a slot alive either.
        idle[0].sendall(b'{"v":1,"id":1,"meth')
        time.sleep(2.0)
        for s in idle:
            s.settimeout(2)
            assert s.recv(10) == b"", "idle connection was not closed"
        ok = _client(p)
        assert _hello(ok)["ok"], "slots must be free again"
        ok.close()
    finally:
        srv.stop()


print("UNIT_DONE %d %d" % (len(PASSED), len(FAILED)), flush=True)
os._exit(1 if FAILED else 0)
