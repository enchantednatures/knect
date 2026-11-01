//! Clap surface and subcommand dispatch.

pub mod diff;
pub mod graph;
pub mod install;
pub mod k9s;
pub mod tap;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::client::{ClientError, resolve_endpoint};

/// Root CLI.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "knect",
    bin_name = "knect",
    version,
    about = "Knative topology explorer"
)]
pub struct Cli {
    /// Unix domain socket path (default `/tmp/knectd.sock`). Ignored when
    /// `--daemon-url` is supplied.
    #[arg(long, global = true, env = "KNECT_SOCKET")]
    pub socket: Option<PathBuf>,
    /// Daemon HTTP base URL, for example `http://127.0.0.1:7420`. Takes
    /// precedence over `--socket`.
    #[arg(long, global = true, env = "KNECT_DAEMON_URL")]
    pub daemon_url: Option<String>,
    /// Render the topology with ASCII box characters instead of Unicode.
    #[arg(long, global = true, env = "KNECT_ASCII")]
    pub ascii: bool,
    /// Optional subcommand. No subcommand opens the dashboard.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// CLI subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Render the topology graph.
    Graph(graph::GraphArgs),
    /// Show mutation attribution for one resource.
    Diff(diff::DiffArgs),
    /// Start or inspect a CloudEvent tap.
    Tap(tap::TapArgs),
    /// Install the k9s plugin stub.
    InstallK9sPlugin(k9s::InstallK9sPluginArgs),
}

/// Typed CLI failure.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The subcommand exists but its implementation lands in a later task.
    #[error("{0} is not implemented yet")]
    NotImplemented(&'static str),
    /// Endpoint resolution failed.
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Dispatches a parsed subcommand.
pub fn run(
    command: Command,
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<(), CliError> {
    let _endpoint = resolve_endpoint(socket, daemon_url)?;
    match command {
        Command::Graph(args) => graph::run(args),
        Command::Diff(args) => diff::run(args),
        Command::Tap(args) => tap::run(args),
        Command::InstallK9sPlugin(args) => k9s::run(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_without_a_subcommand() {
        let cli = Cli::try_parse_from(["knect"]).expect("parse");
        assert!(cli.command.is_none());
        assert!(cli.socket.is_none());
    }

    #[test]
    fn parses_graph_and_install_subcommands() {
        let cli = Cli::try_parse_from(["knect", "graph", "--json"]).expect("parse graph");
        assert!(matches!(cli.command, Some(Command::Graph(_))));
        let cli = Cli::try_parse_from([
            "knect",
            "--socket",
            "/tmp/test.sock",
            "install-k9s-plugin",
            "--force",
        ])
        .expect("parse install");
        assert!(matches!(cli.command, Some(Command::InstallK9sPlugin(_))));
        assert_eq!(cli.socket.as_deref(), Some(Path::new("/tmp/test.sock")));
    }

    #[test]
    fn stubs_return_typed_not_implemented() {
        for command in [
            Command::Graph(graph::GraphArgs {
                json: false,
                resource: None,
                namespace: None,
            }),
            Command::Diff(diff::DiffArgs {
                resource: None,
                mode: diff::DiffModeCli::ThreeWay,
            }),
            Command::Tap(tap::TapArgs {
                target: None,
                follow: false,
            }),
            Command::InstallK9sPlugin(k9s::InstallK9sPluginArgs {
                dir: None,
                force: false,
            }),
        ] {
            match run(command, None, None) {
                Err(CliError::NotImplemented(_)) => {}
                other => panic!("unexpected result: {other:?}"),
            }
        }
    }
}
