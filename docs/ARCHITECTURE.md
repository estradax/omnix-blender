# Architecture

## The constraint that shapes everything

Blender's API (`bpy`) exists only inside Blender's embedded Python. It is not thread-safe and must be driven from
Blender's main thread, so a Rust program cannot call it. The system is therefore two parts:

```
MCP client --stdio--> omnix-blender (Rust, rmcp)  --Unix socket, NDJSON-->  Blender add-on (Python)
                                                                            |  socket threads: never touch bpy
                                                                            v  queue
                                                                            main thread: pump() -> handlers -> bpy
```

* **Rust server** (`src/`): speaks MCP over stdio, owns the tool schemas and annotations, validates arguments
  (`deny_unknown_fields`), and talks to the add-on. `serve` answers the MCP handshake immediately and connects on
  first use, so it starts fine with Blender closed. Logs go to stderr only (stdout is the MCP stream).
* **Add-on** (`addon/omnix_blender/`, pure Python, no third-party dependencies, packaged as a Blender extension and
  embedded in the binary by `build.rs`): a socket server on background threads plus a dispatcher whose `pump()` runs
  queued requests on the main thread.

## Protocol (Rust <-> add-on)

Newline-delimited JSON, one request and one response per line, matched by `id`. The client sends `hello` first; the
add-on never speaks unsolicited and executes nothing before `hello` and the protocol-version check. Responses carry
`ok`/`result` or `ok:false` with a stable error `code` (`not_found`, `invalid_argument`, `unsupported`, `disabled`,
`blender_error`, `timeout`, `busy`, `protocol_error`). Lines are capped at 16 MiB. Requests are serial in v1: while one
is outstanding another gets `busy`. Large binary results (renders) are written to a private temp dir and returned
as a path; the Rust side re-validates the file (location, owner, mode) before reading and returning an image block.

## Threading model (the part most likely to go wrong)

* Socket threads only read/write sockets and enqueue work. They never import or touch `bpy`.
* `Dispatcher.submit()` hands a request to a queue; `pump()` pops and executes it **on the main thread**. In the GUI a
  `bpy.app.timers` callback calls `pump()` every 20 ms; headless (`--background` has no event loop) a loop calls it
  directly, which is also how the tests run the same code path.
* A handler that runs past its client timeout cannot be interrupted (Python). The client gets `timeout`, later
  requests get `busy` until it finishes. Keep handlers short; long renders block the UI by design.
* Prefer the data API (`bpy.data`, `bmesh`) over `bpy.ops`; operators are used only where no data-API route exists,
  with a context override. Every mutating call becomes one undo step.

## Why a Unix socket and not a TCP port

`run_python` is remote code execution by design. A loopback TCP port is reachable by any local process and by web
pages (DNS rebinding and similar tricks), which is the well-known weakness of existing Blender MCP add-ons. A Unix
socket in a `0700` directory with `0600` permissions is reachable only by the same user. Details and the exact checks
are in [SECURITY.md](../SECURITY.md); the add-on never binds a TCP port, and a test asserts it with `lsof`.

## Layers of protection around code and files

1. `run_python` is off unless the user ticks it in the add-on preferences; it is re-checked on every call and the tool
   is annotated destructive so an omnix policy can deny it (`"deny": ["blender.run_python"]`).
2. File tools are a write primitive that does not need `run_python`, so they have their own gate: a path policy
   (resolved path; allowed roots; no hidden or protected locations; no overwrite without `overwrite=true`; extension
   must agree with the format). `open_file` never auto-runs scripts embedded in a `.blend`.
3. Annotations (`readOnlyHint` / `destructiveHint`) are asserted for every tool in `tests/annotations.rs`, which is what
   lets omnix's `read_only` mode expose only the query tools.

## Roadmap (not built)

* `serve-headless`: spawn `blender --background --python addon/headless_runner.py` and connect as for the GUI (the
  runner already exists and is what the tests use).
* Linux CI with Blender installed (`OMNIX_BLENDER_REQUIRE=1`), Windows support (needs a TCP + token fallback, an
  explicit opt-in).
* Phase-2 tools: animation and keyframes, shape keys, UV unwrap, geometry-nodes inputs, world/HDRI, baking, texture
  assignment, a generic RNA `set_property` with an allowlist, `list_addons`.
* Verify on Blender 4.2 LTS; the add-on targets 4.2+ but only 5.3.0 has been tested.
