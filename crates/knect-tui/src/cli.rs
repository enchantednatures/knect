//! Clap surface and subcommand dispatch.

pub mod diff;
pub mod graph;
pub mod install;
pub mod k9s;
pub mod kind;
pub mod tap;
pub mod target;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::client::ClientError;

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
    /// Invalid input or request.
    #[error("{0}")]
    Message(String),
    /// The resource was not found.
    #[error("resource not found: {0}")]
    NotFound(String),
    /// The resource argument matched more than one node.
    #[error("ambiguous resource: {0}")]
    Ambiguous(String),
    /// Endpoint resolution or transport failed.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// File access failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Encodes a deterministic pretty JSON document.
pub fn json_text<T: serde::Serialize>(value: &T) -> Result<String, CliError> {
    serde_json::to_string_pretty(value)
        .map_err(|error| CliError::Message(format!("cannot encode JSON: {error}")))
}

/// Dispatches a parsed subcommand.
pub async fn run(
    command: Command,
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<(), CliError> {
    match command {
        Command::Graph(args) => graph::run(args, socket, daemon_url).await,
        Command::Diff(args) => diff::run(args, socket, daemon_url).await,
        Command::Tap(args) => tap::run(args, socket, daemon_url).await,
        Command::InstallK9sPlugin(args) => k9s::run(args, socket),
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
            "--dry-run",
        ])
        .expect("parse install");
        assert!(matches!(cli.command, Some(Command::InstallK9sPlugin(_))));
        assert_eq!(cli.socket.as_deref(), Some(Path::new("/tmp/test.sock")));
    }

    #[test]
    fn parses_focused_diff_and_tap_flags() {
        let cli = Cli::try_parse_from([
            "knect",
            "diff",
            "ksvc/hello",
            "-n",
            "default",
            "--json",
            "--mode",
            "managers",
        ])
        .expect("parse diff");
        match cli.command {
            Some(Command::Diff(args)) => {
                assert_eq!(args.namespace.as_deref(), Some("default"));
                assert!(args.json);
                assert_eq!(args.mode, diff::DiffModeCli::Managers);
            }
            other => panic!("unexpected command: {other:?}"),
        }
        let cli = Cli::try_parse_from(["knect", "tap", "broker/orders", "-n", "default", "--list"])
            .expect("parse tap");
        match cli.command {
            Some(Command::Tap(args)) => {
                assert!(args.list);
                assert_eq!(args.namespace.as_deref(), Some("default"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
