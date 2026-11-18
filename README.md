<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
    <img src="assets/logo-light.svg" alt="omnix-blender" width="430">
  </picture>
</p>

<p align="center">
  <b>Drive a live Blender session over MCP.</b><br>
  A Rust MCP server plus a small Python add-on, connected by a private Unix socket.<br>
  Part of the <a href="https://github.com/estradax/omnix">omnix</a> family.
</p>

<p align="center">
  <a href="https://github.com/estradax/omnix-blender/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/estradax/omnix-blender/actions/workflows/ci.yml/badge.svg"></a>
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue">
  <img alt="Blender 5.3" src="https://img.shields.io/badge/blender-5.3-orange">
  <img alt="Status: alpha" src="https://img.shields.io/badge/status-alpha-orange">
</p>

---

# omnix-blender

A Rust [MCP](https://modelcontextprotocol.io) server that controls a **live Blender session**. It talks to a small
pure-Python Blender add-on over a **Unix domain socket** (newline-delimited JSON). Works with any MCP client and sits
behind [omnix](https://github.com/estradax/omnix) without special support. 

```
MCP client --stdio--> omnix-blender (Rust, rmcp) --Unix socket, NDJSON--> Blender add-on --main-thread queue--> bpy
```

## Install and try it

```sh
cargo build --release                      # binary: target/release/omnix-blender
omnix-blender install-addon                # installs + enables the bundled add-on via Blender's own CLI
```

1. Open Blender 5.3 (4.2+ should work, see "Tested").
2. In the 3D viewport press `N`, open the **Omnix** tab, click **Start**. (Auto-start is off by default.)
3. `omnix-blender doctor` should print all `[ok]`.
4. Add the server to your MCP client:

```json
{ "mcpServers": { "blender": { "command": "/absolute/path/to/omnix-blender", "args": ["serve"] } } }
```

Behind omnix: `omnix add blender --command /absolute/path/to/omnix-blender` (then e.g. `"policy": {"read_only": true}`
exposes only the query tools; `"deny": ["blender.run_python"]` blocks the escape hatch).

`omnix-blender install-addon --uninstall` removes it again. Both are idempotent. `install-addon` changes the Blender
user profile of whoever runs it (that is the point); the tests only ever run it against a temporary profile.

`serve` answers the MCP handshake immediately and only connects to Blender on the first tool call, so it starts fine
when Blender is closed; tool calls then return an actionable error instead of hanging.

## Tools

Query (readOnlyHint): `blender_guide`, `scene_info`, `list_objects`, `get_object`, `viewport_screenshot` (GUI only).
Create/modify: `create_primitive`, `create_object`, `set_transform`, `duplicate_object`, `set_parent`, `select_objects`,
`collection`, `set_material`, `add_modifier`, `set_render_settings`, `render_image`, `import_file`, `undo`, `redo`.
Destructive (destructiveHint): `delete_objects`, `apply_modifier`, `remove_modifier`, `save_file`, `open_file`,
`new_file`, `export_file`, `run_python`.
`run_python` is **off** until you tick *Allow run_python* in the add-on preferences (or the N panel); the tool error says so.
Units are metres, Z-up; tool rotations are degrees. Every mutating call is one undo step.

## Security model

* Unix socket only, no TCP port. Directory `0700`, socket `0600`; the add-on refuses a group/world-writable or
  foreign-owned directory, refuses to replace a non-socket file, replaces a stale socket, and never steals a live one.
  The client refuses an untrusted socket directory. See [SECURITY.md](SECURITY.md).
* Nothing executes before `hello` and the protocol version check. Lines are capped at 16 MiB.
* `run_python` is remote code execution by design, hence off by default and re-checked on every call.
* File tools only touch the saved `.blend`'s folder and the add-on preference *Allowed folders* (default empty), never
  hidden paths or protected locations, never overwrite without `overwrite=true`, and `open_file` never runs embedded
  scripts. To work in a folder: save the .blend there, add it under *Allowed folders* in the add-on preferences, or (less
  safe) tick *Allow file access outside allowed folders*.

## Tested (and not)

Verified on **Blender 5.3.0, macOS 15.7 arm64**, by `cargo test` (needs Blender; tests skip with a message if it is not
found, `OMNIX_BLENDER_BIN` overrides discovery). The suite drives the real chain: rmcp client -> built binary -> socket
-> real headless Blender running the real add-on, in an isolated profile (`--factory-startup`, temp
`BLENDER_USER_RESOURCES`/`BLENDER_USER_SCRIPTS`), asserting actual `bpy` state. Also: install/uninstall/enable/disable
lifecycle with the installed extension, `doctor`, crash/reconnect, stale sockets, malformed/oversized input, no TCP
listener, busy/timeout, and an omnix `read_only` policy run (skipped when `~/project/omnix/target/debug/omnix` is absent).
Set `OMNIX_BLENDER_REQUIRE=1` in CI that is supposed to have Blender: a missing Blender (or `lsof`) then fails the run
instead of skipping. Add-on socket/server unit tests run inside Blender (`tests/support/unit_addon.py`).
One ignored test, `cargo test --test e2e_security -- --ignored gui_smoke`, opens a real Blender window for a few seconds
(timer-driven pump, undo from a timer, screenshot, render, apply modifier, import/export, save).

**Not verified:** Linux and Windows; Blender versions other than 5.3.0; behaviour while Blender is minimised, under
App Nap, or while a modal operator runs (timers may throttle: keep Blender visible); interactive use of the N panel
buttons by a human (the operators were exercised, the clicks were not); `INVOKE_DEFAULT` rendering (we use a blocking
render); very large scenes/renders.

Known limits: a render or any long handler blocks Blender's UI and its single request queue (other requests answer
`busy`, a timed-out request keeps running because Python cannot be interrupted); `unsaved` tracking is best effort
(headless it counts changes made through the tools); `viewport_screenshot` needs the GUI.

Design: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Decisions and deviations: [docs/DECISIONS.md](docs/DECISIONS.md). Measured facts about Blender 5.3: [docs/FINDINGS.md](docs/FINDINGS.md).

License: MIT OR Apache-2.0.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Report security problems as described in [SECURITY.md](SECURITY.md).
