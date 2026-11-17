# omnix-blender — instructions for the implementing agent

Read `PLAN.md` first; it is the full brief and the source of truth.

Short version:

- Build a **Rust MCP server** (`rmcp` 3.10, stdio) that controls **Blender 5.3** through a small **Python add-on**
  over a **Unix domain socket** with newline-delimited JSON. Blender's `bpy` is main-thread-only, so all execution goes
  through a queue drained by `pump()` on Blender's main thread.
- Reference for the rmcp server pattern: `~/project/omnix/examples/mock_mcp.rs` (read only; do not edit that repo).
- Blender is at `/Applications/Blender.app/Contents/MacOS/Blender` (5.3.0). Always run it with `--factory-startup` and an
  isolated `BLENDER_USER_RESOURCES` / `BLENDER_USER_SCRIPTS` so the owner's real setup is never touched.
- Start with **M0** (verify unknowns, write `docs/FINDINGS.md`), then M1 → M3 in order, and **stop and report after M3**.
- Private, local-only repo. No remotes, no publishing. Commit locally after each milestone.
- Quality gate before every commit: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- macOS has no `timeout` command; use `perl -e 'alarm N; exec @ARGV' <cmd>`.
- Be honest about what is and is not verified. Trust observation over the plan when they conflict, and record it.
