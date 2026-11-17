# Security notes

omnix-blender gives an MCP client control of a Blender session. Treat the client as trusted with the things below,
and use the omnix `read_only` / `deny` policies to take capabilities away.

## Transport

* Unix domain socket only; the add-on never opens a TCP/UDP port (checked in the tests with `lsof`).
* The socket directory must be owned by you, must **not be a symlink**, and must have **no group/other permission
  bits** (mode `0700`). A missing directory is created `0700`. An existing directory that does not meet this is
  *refused* with the `chmod 700` hint; it is never silently chmod'ed. The client applies the same rule before connecting.
* The socket is bound inside that private directory and chmod'ed `0600` immediately; no process-wide `umask` change is
  used. Because the directory is private, nobody else can reach the file in the instant between `bind` and `chmod`.
* A stale socket (connection refused) is replaced. If liveness cannot be established (timeout, permission error,
  full backlog) the add-on refuses to start and **leaves the file alone**. A live listener is never replaced. A path
  that exists but is not a socket is never replaced.
* No peer-credential check: anything running as you can connect. Nothing executes before `hello` and the
  protocol-version check. Lines are capped at 16 MiB, unknown request fields/params are rejected, `timeout_s` must be
  a finite number > 0, at most 8 connections are served and an idle connection (no complete request for 10 min) is dropped.

## run_python

Executes arbitrary Python (with `os`/`subprocess`) in Blender. Off by default, enabled by the add-on preference
*Allow run_python*, re-read on every call. Deny it in omnix policy if you never want it. It is outside the file
policy below (it can do anything Blender can).

## File access (save_file, export_file, import_file, open_file, render output_path)

Every path is resolved with `realpath` (symlinks followed) **before** it is checked:

1. Hidden components (any component starting with `.`) are always refused, in the given and the resolved path.
2. A protected list is always refused: `~/.ssh`, `~/.gnupg`, `~/.config`, `~/Library/LaunchAgents`,
   `~/Library/Keychains`, shell rc/profile files, `/etc`, `/System`, `/Library`, `/usr`, `/bin`, `/sbin`.
3. The path must be inside an allowed root: the folder of the saved `.blend`, a folder from the add-on preference
   *Allowed folders* (default empty), or the add-on's private render directory. Otherwise it is refused (code
   `disabled`, with the way to allow it) unless the preference *Allow file access outside allowed folders* is on
   (default **off**; also in the N panel). Rules 1 and 2 hold even then.
4. Replacing an existing file requires `overwrite=true` (`save_file`, `export_file`; the open file may be re-saved in place).
5. `export_file` requires a known extension that matches `format`; side files the exporter writes (`.mtl`, `.bin`) get
   the same checks up front and are listed in the result (`side_files`).

Limits: this is a path check, not a sandbox. A time-of-check/time-of-use race against a *same-user* process that swaps
symlinks is not prevented; exporter file *contents* are produced by Blender (object names end up inside text formats such
as `.obj`/`.mtl`, so do not `source` an exported file). With *Allow file access outside allowed folders* on, any
user-writable, non-hidden, non-protected file can be read or (with `overwrite=true`) replaced.

## open_file and embedded scripts

`.blend` files can carry scripts (text blocks flagged to run on load, drivers). `open_file` always passes
`use_scripts=False`, so embedded scripts do not run even if your Blender profile trusts auto-execution (regression
test: `open_file_never_auto_executes_embedded_scripts`). This protects the `open_file` tool only; opening an untrusted
`.blend` yourself in the GUI is governed by Blender's own settings.

## Renders and screenshots

Images are written to a private `0700` `omnix-render-*` directory under the system temp dir. The server only reads an
image that is a regular file owned by you, directly inside such a directory that is under its own temp dir, owned by you,
mode `0700`, not a symlink; the file is opened once with `O_NOFOLLOW` and checked through the descriptor. (The add-on and
the server must see the same `TMPDIR`.)

## Other

A request cannot be interrupted once it runs inside Blender; a client can block the UI with a long operation (same
user only).

Report problems privately to the repository owner.
