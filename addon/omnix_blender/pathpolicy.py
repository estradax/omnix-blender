"""Path policy for every tool that reads or writes a file.

Rules (checked on the *resolved* path, symlinks followed, before anything is touched):
 1. Hidden path components (any component starting with '.') are always refused.
 2. A built-in deny list of sensitive locations is always refused (see ``denied_locations``).
 3. The path must lie inside an allowed root: the folder of the current .blend, a folder from the
    'Allowed folders' preference, or the private render temp dir. Otherwise it is refused with code
    ``disabled`` unless the preference 'Allow file access outside allowed folders' is on.
 4. Writing over an existing file needs an explicit ``overwrite=true`` (callers enforce this).
Rules 1 and 2 hold even when rule 3 is switched off.
"""

import os

from .protocol import DISABLED, INVALID_ARGUMENT, OmnixError

_HOME_REL_DENIED = (
    ".ssh", ".gnupg", ".config", "Library/LaunchAgents", "Library/Keychains",
    ".zshrc", ".zprofile", ".zshenv", ".zlogin", ".zlogout",
    ".bashrc", ".bash_profile", ".bash_login", ".bash_logout", ".profile",
)
_ABS_DENIED = ("/etc", "/private/etc", "/System", "/Library", "/usr", "/bin", "/sbin")

HOW_TO_ALLOW = (
    "add the folder to 'Allowed folders' in Edit > Preferences > Add-ons > Omnix Blender, save the .blend into the "
    "target folder first, or (less safe) tick 'Allow file access outside allowed folders' there or in the N panel > Omnix"
)


def denied_locations():
    home = os.path.realpath(os.path.expanduser("~"))
    out = [os.path.join(home, rel) for rel in _HOME_REL_DENIED]
    out += [os.path.realpath(p) for p in _ABS_DENIED]
    return out


def _inside(path, root):
    return path == root or path.startswith(root.rstrip(os.sep) + os.sep)


def allowed_roots(ctx):
    import bpy

    roots = []
    if bpy.data.filepath:
        roots.append(os.path.dirname(os.path.realpath(bpy.data.filepath)))
    for f in ctx.allowed_folders():
        if f:
            roots.append(os.path.realpath(os.path.expanduser(f)))
    from .handlers import render

    if render.render_dir_if_any():
        roots.append(os.path.realpath(render.render_dir_if_any()))
    return roots


def _components(path):
    return [c for c in path.split(os.sep) if c]


def resolve(raw, ctx, what="path", allow_pattern_basename=False):
    """Validate ``raw`` and return the resolved absolute path to operate on. Raises OmnixError."""
    if not isinstance(raw, str) or not raw.strip():
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be a non-empty string" % what)
    if "\x00" in raw:
        raise OmnixError(INVALID_ARGUMENT, "'%s' contains a NUL byte" % what)
    p = os.path.expanduser(raw)
    if not os.path.isabs(p):
        raise OmnixError(INVALID_ARGUMENT, "'%s' must be an absolute path (got %r)" % (what, raw))
    lexical = os.path.abspath(p)
    if allow_pattern_basename and not raw.endswith(os.sep):
        head, tail = os.path.split(lexical)
        real = os.path.join(os.path.realpath(head), tail)
    else:
        real = os.path.realpath(lexical)
    for label, candidate in (("given", lexical), ("resolved", real)):
        for comp in _components(candidate):
            if comp.startswith("."):
                raise OmnixError(
                    DISABLED,
                    "refused: the %s path of '%s' contains the hidden component '%s' (%s). Hidden folders and files are "
                    "never accessible, whatever the preferences" % (label, what, comp, candidate),
                )
    for d in denied_locations():
        if _inside(real, d) or _inside(lexical, d):
            raise OmnixError(
                DISABLED,
                "refused: '%s' is inside a protected location (%s). Protected locations are never accessible, "
                "whatever the preferences" % (what, d),
            )
    if not ctx.allow_outside_files():
        roots = allowed_roots(ctx)
        if not any(_inside(real, r) for r in roots):
            raise OmnixError(
                DISABLED,
                "refused: %s is outside the folders Omnix may access. To allow it: %s. Currently allowed: %s"
                % (real, HOW_TO_ALLOW, ", ".join(roots) or "none (the scene has no saved file and no 'Allowed folders' are set)"),
            )
    return real


def require_writable_target(real, overwrite, what="path"):
    if os.path.isdir(real):
        raise OmnixError(INVALID_ARGUMENT, "'%s' is a directory" % what)
    if os.path.lexists(real) and not overwrite:
        raise OmnixError(
            INVALID_ARGUMENT,
            "%s already exists (%s); refusing to overwrite it. Pass overwrite=true to replace it" % (real, what),
        )
