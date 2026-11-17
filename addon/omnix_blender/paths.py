"""Socket path resolution and the security checks around it. No ``bpy`` here."""

import os
import socket
import stat
import sys

from .protocol import MAX_SOCKET_PATH, OmnixError, INVALID_ARGUMENT


def config_dir():
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support")
    if sys.platform.startswith("win"):
        return os.environ.get("APPDATA") or os.path.expanduser("~")
    return os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")


def default_socket_path():
    env = os.environ.get("OMNIX_BLENDER_SOCKET")
    if env:
        return env
    return os.path.join(config_dir(), "omnix-blender", "blender.sock")


def prepare_socket_path(path):
    """Validate ``path`` and its directory; remove a stale socket. Raises OmnixError on problems."""
    if len(os.fsencode(path)) > MAX_SOCKET_PATH:
        raise OmnixError(
            INVALID_ARGUMENT,
            "socket path is %d bytes; Unix socket paths are limited to %d. Set OMNIX_BLENDER_SOCKET to a shorter path."
            % (len(os.fsencode(path)), MAX_SOCKET_PATH),
        )
    directory = os.path.dirname(os.path.abspath(path))
    if not os.path.isdir(directory):
        os.makedirs(os.path.dirname(directory), exist_ok=True)
        os.mkdir(directory, 0o700)
    st = os.stat(directory)
    if hasattr(os, "getuid") and st.st_uid != os.getuid():
        raise OmnixError(INVALID_ARGUMENT, "socket directory %s is not owned by you; refusing to start" % directory)
    if st.st_mode & (stat.S_IWGRP | stat.S_IWOTH):
        raise OmnixError(
            INVALID_ARGUMENT,
            "socket directory %s is group/world writable (mode %o); chmod 700 it or choose another path"
            % (directory, stat.S_IMODE(st.st_mode)),
        )
    try:
        st = os.lstat(path)
    except FileNotFoundError:
        return
    if not stat.S_ISSOCK(st.st_mode):
        raise OmnixError(INVALID_ARGUMENT, "%s exists and is not a socket; refusing to replace it" % path)
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    probe.settimeout(1.0)
    try:
        probe.connect(path)
    except (ConnectionRefusedError, FileNotFoundError):
        os.unlink(path)  # stale socket left by a crashed run
        return
    except OSError:
        os.unlink(path)
        return
    finally:
        probe.close()
    raise OmnixError(INVALID_ARGUMENT, "another Blender session is already listening on %s" % path)
