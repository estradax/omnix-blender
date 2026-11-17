use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use omnix_blender::{blender, doctor, link::Link, mcp::OmnixBlender, paths};
use rmcp::{ServiceExt, transport::stdio};
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "omnix-blender",
    version,
    about = "MCP server for a live Blender session"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the MCP server on stdio (default). Connects to Blender lazily on first use.
    Serve,
    /// Check Blender, the add-on and the socket; exits non-zero on problems.
    Doctor,
    /// Install the bundled add-on into Blender's user extensions (via Blender's own CLI) and enable it.
    InstallAddon {
        /// Remove the add-on instead.
        #[arg(long)]
        uninstall: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => {
            let path = paths::default_socket_path();
            eprintln!(
                "omnix-blender: MCP server on stdio; Blender socket {}",
                path.display()
            );
            let server = OmnixBlender::new(Arc::new(Link::new(path)));
            let svc = server.serve(stdio()).await.context("MCP handshake")?;
            svc.waiting().await?;
            Ok(())
        }
        Cmd::Doctor => {
            let report = doctor::run().await;
            for l in &report.lines {
                println!("{l}");
            }
            if report.ok {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }
        Cmd::InstallAddon { uninstall } => {
            let Some(bin) = blender::find_blender() else {
                bail!("Blender not found; set OMNIX_BLENDER_BIN");
            };
            let msg = if uninstall {
                blender::uninstall_addon(&bin)?
            } else {
                blender::install_addon(&bin)?
            };
            println!("{msg}");
            Ok(())
        }
    }
}
