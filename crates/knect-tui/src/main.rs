//! `knect` entrypoint.

use clap::Parser;
use knect_tui::cli::{self, Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command.clone() {
        None => {
            if let Err(error) =
                knect_tui::app::run(cli.socket.clone(), cli.daemon_url.clone(), cli.ascii, None)
                    .await
            {
                eprintln!("knect: {error}");
                std::process::exit(1);
            }
        }
        Some(command) => {
            if let Err(error) =
                cli::run(command, cli.socket.as_deref(), cli.daemon_url.as_deref()).await
            {
                eprintln!("knect: {error}");
                std::process::exit(1);
            }
        }
    }
}
