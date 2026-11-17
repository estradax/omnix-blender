# omnix-blender — notes for agents

A **Rust MCP server** (`rmcp` 3.10, stdio) that controls **Blender 5.3** through a small **Python add-on** over a
**Unix domain socket** (newline-delimited JSON). Read `README.md`, `docs/ARCHITECTURE.md` (design, threading model,
why a socket), `SECURITY.md` and `docs/DECISIONS.md` before changing anything.

Rules:

- Private, local-only repo. No remotes, no publishing.
- Blender is at `/Applications/Blender.app/Contents/MacOS/Blender`. Always run it with `--factory-startup` and an isolated
  `BLENDER_USER_RESOURCES` / `BLENDER_USER_SCRIPTS`, so the owner's real preferences and add-ons are never touched.
- `bpy` is main-thread-only: socket threads must never import or touch it; all execution goes through `pump()`.
- Any tool that reads or writes a file must go through `addon/omnix_blender/pathpolicy.py`.
- Quality gate before every commit: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (set `OMNIX_BLENDER_REQUIRE=1` to make a missing Blender fail instead of skip).
- macOS has no `timeout` command; use `perl -e 'alarm N; exec @ARGV' <cmd>`.
- Be honest about what is and is not verified, and add a regression test for every bug fixed.
