# Contributing

Thanks for your interest. omnix-blender is alpha software. The most useful contributions right now are bug reports
from real use and verification on platforms and Blender versions we have not tested (see "Tested (and not)" in the
README: only Blender 5.3.0 on macOS arm64 is verified).

## Setup

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
OMNIX_BLENDER_REQUIRE=1 cargo test     # needs Blender; without REQUIRE a missing Blender skips the e2e tests
```

`OMNIX_BLENDER_BIN` overrides Blender discovery. Tests always run Blender with `--factory-startup` and an isolated
`BLENDER_USER_RESOURCES` / `BLENDER_USER_SCRIPTS`, so your real preferences and add-ons are never touched. Keep it that way.

## Layout

| Path | |
|---|---|
| `src/mcp.rs` | The MCP server and tool definitions. |
| `src/link.rs`, `src/paths.rs` | Socket client and socket-directory trust checks. |
| `src/blender.rs`, `src/doctor.rs` | Blender discovery, `install-addon`, `doctor`. |
| `addon/omnix_blender/` | The Blender add-on: `server.py` (socket threads), `dispatch.py` + `pump()` (main thread), `handlers/`. |
| `tests/` | End-to-end tests against real headless Blender, plus in-Blender add-on unit tests. |

## Guidelines

- `bpy` is main-thread only. Socket threads must never import or touch it; everything runs through `pump()`.
- Any tool that reads or writes a file must go through `addon/omnix_blender/pathpolicy.py`.
- Add a regression test for every bug fixed, and be honest in the docs about what is and is not verified.
- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [SECURITY.md](SECURITY.md) before changing the protocol or file handling.
- By contributing you agree your work is licensed under the project's `MIT OR Apache-2.0` terms.
