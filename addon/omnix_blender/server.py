"""Unix-domain-socket server. Runs on background threads and never touches ``bpy``."""

import math
import os
import socket
import threading
import time

from . import paths, protocol
from .protocol import OmnixError


MAX_CONNECTIONS = 8
IDLE_TIMEOUT = 600.0  # seconds without a complete request before a connection is dropped


class Server:
    def __init__(self, path, dispatcher, info, idle_timeout=IDLE_TIMEOUT, max_connections=MAX_CONNECTIONS):
        self.idle_timeout = idle_timeout
        self.max_connections = max_connections
        self.path = path
        self.dispatcher = dispatcher
        self.info = info  # dict returned by hello (static values captured on the main thread)
        self._listener = None
        self._thread = None
        self._stop = threading.Event()
        self._conns = set()
        self._threads = []
        self._conns_lock = threading.Lock()
        self.connections_served = 0

    def start(self):
        paths.prepare_socket_path(self.path)
        # No process-wide umask games. The directory is private (0700, checked above), so no other user can reach
        # the socket file in the instant between bind() and chmod(); the chmod makes the mode explicit.
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.bind(self.path)
            os.chmod(self.path, 0o600)
        except BaseException:
            sock.close()
            raise
        sock.listen(4)
        sock.settimeout(0.25)
        self._listener = sock
        self._thread = threading.Thread(target=self._accept_loop, name="omnix-accept", daemon=True)
        self._thread.start()

    def stop(self):
        """Shut everything down in about a second at most (this runs on Blender's main thread)."""
        deadline = time.monotonic() + 1.5
        self._stop.set()
        if self._listener is not None:
            try:
                self._listener.close()
            except OSError:
                pass
        with self._conns_lock:
            conns = list(self._conns)
        for c in conns:
            try:
                c.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        for t in [self._thread] + list(self._threads):
            if t is not None:
                t.join(timeout=max(0.0, deadline - time.monotonic()))
        try:
            if os.path.exists(self.path) or os.path.islink(self.path):
                os.unlink(self.path)
        except OSError:
            pass

    def _accept_loop(self):
        while not self._stop.is_set():
            try:
                conn, _ = self._listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            conn.settimeout(0.5)
            with self._conns_lock:
                if len(self._conns) >= self.max_connections:
                    # Same-user clients only, but still bound the thread count.
                    self._send(conn, protocol.err(None, protocol.BUSY, "too many connections"))
                    conn.close()
                    continue
                self._conns.add(conn)
                self._threads = [t for t in self._threads if t.is_alive()]
                t = threading.Thread(target=self._serve, args=(conn,), name="omnix-conn", daemon=True)
                self._threads.append(t)
            self.connections_served += 1
            t.start()

    def _send(self, conn, message):
        try:
            conn.sendall(protocol.encode(message))
            return True
        except OSError:
            return False

    def _serve(self, conn):
        buf = b""
        greeted = False
        last_activity = time.monotonic()  # time of the last *complete* request (or of connecting)
        try:
            while not self._stop.is_set():
                if time.monotonic() - last_activity > self.idle_timeout:
                    return  # idle (or stalled mid-line): free the slot
                try:
                    chunk = conn.recv(65536)
                except socket.timeout:
                    continue
                except OSError:
                    return
                if not chunk:
                    return
                buf += chunk
                while True:
                    nl = buf.find(b"\n")
                    if nl < 0:
                        if len(buf) > protocol.MAX_LINE:
                            self._send(conn, protocol.err(None, protocol.INVALID_ARGUMENT, "request line exceeds 16 MiB"))
                            return
                        break
                    line, buf = buf[:nl], buf[nl + 1 :]
                    if len(line) > protocol.MAX_LINE:
                        self._send(conn, protocol.err(None, protocol.INVALID_ARGUMENT, "request line exceeds 16 MiB"))
                        return
                    if not line.strip():
                        continue
                    greeted = self._handle_line(conn, line, greeted)
                    if greeted is None:
                        return
                    last_activity = time.monotonic()
        finally:
            with self._conns_lock:
                self._conns.discard(conn)
            try:
                conn.close()
            except OSError:
                pass

    def _handle_line(self, conn, line, greeted):
        """Returns the new ``greeted`` flag, or None to close the connection."""
        try:
            req = protocol.decode_request(line)
        except OmnixError as exc:
            self._send(conn, protocol.err(None, exc.code, exc.message))
            # A version mismatch, or any bad request before hello, ends the conversation; later errors do not.
            return None if (not greeted or exc.code == protocol.UNSUPPORTED) else greeted
        req_id = req.get("id")
        if req["method"] == "hello":
            self._send(conn, protocol.ok(req_id, dict(self.info)))
            return True
        if not greeted:
            self._send(conn, protocol.err(req_id, protocol.PROTOCOL_ERROR, "send 'hello' first"))
            return None
        timeout = req.get("timeout_s", protocol.DEFAULT_TIMEOUT_S)
        if (
            isinstance(timeout, bool)
            or not isinstance(timeout, (int, float))
            or not math.isfinite(timeout)
            or timeout <= 0
        ):
            self._send(conn, protocol.err(req_id, protocol.INVALID_ARGUMENT, "timeout_s must be a finite number > 0"))
            return greeted
        pending = self.dispatcher.submit(req)
        if pending is None:
            resp = protocol.err(
                req_id, protocol.BUSY, "Blender is still working on a previous request; retry shortly"
            )
        else:
            resp = self.dispatcher.wait(pending, min(float(timeout), 3600.0), self._stop)
        return greeted if self._send(conn, resp) else None
