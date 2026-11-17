# Security notes

omnix-blender gives an MCP client control of a Blender session. Treat the client as trusted with your user's
Blender and file permissions.

* **Transport**: Unix domain socket in a `0700` directory, socket `0600`, same user only. The add-on never opens a
  TCP/UDP port. There is no authentication beyond filesystem permissions; there is no peer-credential check, so
  anything running as you can connect (that is also true of the Blender process itself).
* **run_python** executes arbitrary Python (including `os`/`subprocess`) in Blender. Off by default, enabled only by the
  add-on preference, re-read on every call. Mark it denied in omnix policy if you never want it.
* **File tools** (`save_file`, `open_file`, `import_file`, `export_file`, render output) read/write any path the
  Blender process can, with no allowlist, and are *not* gated by the run_python preference. `export_file` and
  `save_file` overwrite. Use the omnix `read_only` policy to hide them.
* **Renders** are written to a private (`0700`) `omnix-render-*` temp directory; the server only reads images from
  such a directory.
* A request cannot be interrupted once it is running inside Blender; a hostile or buggy client can block the UI by
  sending a long operation (but only as the same user).

Report problems privately to the repository owner.
