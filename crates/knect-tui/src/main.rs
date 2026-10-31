//! `knect` entrypoint.

use clap::Parser;
use knect_tui::cli::{self, Cli};

fn main() {
    let cli = Cli::parse();
    match cli.command.clone() {
        None => {
            println!("TUI implementation pending. Use --help to see CLI commands.");
        }
        Some(command) => {
            if let Err(error) = cli::run(command, cli.socket.as_deref(), cli.daemon_url.as_deref())
            {
                eprintln!("knect: {error}");
                std::process::exit(1);
            }
        }
    }
}
