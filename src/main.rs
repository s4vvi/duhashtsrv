use anyhow::Result;
use clap::Parser;

mod utils;
mod args;
mod logger;
mod globals;
mod server;
mod proto;

use args::Args;

#[tokio::main]
async fn main() -> Result<()> {
    let cmdline = Args::parse();
    let mut server = server::Server::new(cmdline);
    server.start().await;
    Ok(())
}
