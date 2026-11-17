"""Wire protocol: newline-delimited JSON, one request per line, one response per line.

This module must not import ``bpy`` so it can be unit-tested with plain Python.
"""

import json
import math

PROTOCOL_VERSION = 1
REQUEST_KEYS = frozenset({"v", "id", "method", "params", "timeout_s"})
MAX_LINE = 16 * 1024 * 1024
DEFAULT_TIMEOUT_S = 30.0
# AF_UNIX sun_path is 104 bytes on macOS (103 usable); stay under that everywhere.
MAX_SOCKET_PATH = 103

# Stable error codes.
NOT_FOUND = "not_found"
INVALID_ARGUMENT = "invalid_argument"
UNSUPPORTED = "unsupported"
DISABLED = "disabled"
BLENDER_ERROR = "blender_error"
TIMEOUT = "timeout"
BUSY = "busy"
PROTOCOL_ERROR = "protocol_error"


class OmnixError(Exception):
    """An error with a stable ``code``, raised by handlers and mapped to an error response."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


def _round(value):
    if isinstance(value, float):
        if math.isnan(value) or math.isinf(value):
            return None
        return round(value, 6)
    if isinstance(value, dict):
        return {str(k): _round(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_round(v) for v in value]
    return value


def encode(message):
    """Serialise a message to one compact JSON line (bytes, newline terminated)."""
    text = json.dumps(_round(message), separators=(",", ":"), ensure_ascii=False, default=str)
    return text.encode("utf-8") + b"\n"


def ok(req_id, result):
    return {"v": PROTOCOL_VERSION, "id": req_id, "ok": True, "result": result}


def err(req_id, code, message):
    return {
        "v": PROTOCOL_VERSION,
        "id": req_id,
        "ok": False,
        "error": {"code": code, "message": message},
    }


def decode_request(line):
    """Parse one request line. Raises OmnixError(PROTOCOL_ERROR/INVALID_ARGUMENT) on bad input."""
    try:
        msg = json.loads(line.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as exc:
        raise OmnixError(PROTOCOL_ERROR, "malformed JSON: %s" % exc)
    if not isinstance(msg, dict):
        raise OmnixError(PROTOCOL_ERROR, "request must be a JSON object")
    unknown = sorted(set(msg) - REQUEST_KEYS)
    if unknown:
        raise OmnixError(PROTOCOL_ERROR, "unknown request field(s): %s" % ", ".join(map(str, unknown)))
    if msg.get("v") != PROTOCOL_VERSION:
        raise OmnixError(
            UNSUPPORTED,
            "protocol version %r is not supported (add-on speaks v%d); update the add-on or the server"
            % (msg.get("v"), PROTOCOL_VERSION),
        )
    if not isinstance(msg.get("method"), str):
        raise OmnixError(PROTOCOL_ERROR, "request.method must be a string")
    params = msg.get("params", {})
    if params is None:
        params = {}
    if not isinstance(params, dict):
        raise OmnixError(INVALID_ARGUMENT, "request.params must be an object")
    msg["params"] = params
    return msg
