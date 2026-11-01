//! `knect diff` one-shot mutation attribution.

use std::path::Path;
use std::sync::Arc;

use clap::{Args, ValueEnum};
use knect_proto::mutation::{MutationDiff, MutationDiffMode, MutationSource};
use knect_proto::rpc::GetDiffParams;

use super::target;
use super::{CliError, json_text};
use crate::app::modal::{SOURCES, pane_document, pane_lines};

/// Diff rendering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiffModeCli {
    /// Compare user intent, Knative defaults, and policy mutators.
    ThreeWay,
    /// Group ownership by manager.
    Managers,
}

impl DiffModeCli {
    fn protocol(self) -> MutationDiffMode {
        match self {
            Self::ThreeWay => MutationDiffMode::ThreeWay,
            Self::Managers => MutationDiffMode::Managers,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ThreeWay => "three_way",
            Self::Managers => "managers",
        }
    }
}

/// Arguments for `knect diff`.
#[derive(Debug, Clone, Args)]
pub struct DiffArgs {
    /// Resource as `kind/name` or `apiVersion/kind/namespace/name`.
    pub resource: Option<String>,
    /// Namespace for the resource.
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    /// Diff rendering mode.
    #[arg(long, value_enum, default_value = "three-way")]
    pub mode: DiffModeCli,
    /// Emit JSON instead of a terminal view.
    #[arg(long)]
    pub json: bool,
}

/// Runs the diff subcommand.
pub async fn run(
    args: DiffArgs,
    socket: Option<&Path>,
    daemon_url: Option<&str>,
) -> Result<(), CliError> {
    let Some(resource) = args.resource.as_deref() else {
        return Err(CliError::Message(
            "diff requires a resource: <kind>/<name>".to_owned(),
        ));
    };
    let connected = target::connect(socket, daemon_url).await?;
    let topology = target::fetch_topology(Arc::clone(&connected.client)).await?;
    let parsed = target::parse_target(resource)?;
    let id = target::resolve_target(&topology, &parsed, args.namespace.as_deref())?;
    let params = GetDiffParams {
        id,
        mode: Some(args.mode.protocol()),
    };
    let params = serde_json::to_value(params)
        .map_err(|error| CliError::Message(format!("cannot encode diff params: {error}")))?;
    let value = connected
        .client
        .call(
            knect_proto::rpc::methods::MUTATIONS_GET_DIFF.to_owned(),
            Some(params),
        )
        .await?;
    let diff: MutationDiff = serde_json::from_value(value).map_err(|error| {
        CliError::Client(crate::client::ClientError::Protocol(format!(
            "cannot decode mutation diff: {error}"
        )))
    })?;
    if args.json {
        println!("{}", json_text(&diff)?);
    } else {
        print_diff(&diff, args.mode);
    }
    Ok(())
}

fn print_diff(diff: &MutationDiff, mode: DiffModeCli) {
    println!("resource: {}", diff.resource);
    println!("mode: {}", mode.label());
    for source in SOURCES {
        let (label, _) = source_label(source);
        println!("== {label} ==");
        let lines = pane_lines(diff, source);
        if lines.is_empty() {
            match pane_document(diff, source) {
                Some(document) => {
                    for (index, line) in document.lines().enumerate() {
                        println!("{:>4}   {}", index + 1, line);
                    }
                }
                None => println!("    (no lines)"),
            }
            continue;
        }
        for line in lines {
            let marker = if line.changed { "*" } else { " " };
            println!(
                "{:>4} {} {:<32} {}",
                line.line_number,
                marker,
                line.owner.as_deref().unwrap_or("-"),
                line.text
            );
        }
    }
}

fn source_label(source: MutationSource) -> (&'static str, &'static str) {
    match source {
        MutationSource::UserIntent => ("user_intent", "user"),
        MutationSource::KnativeDefaults => ("knative_defaults", "knative"),
        MutationSource::PolicyMutators => ("policy_mutators", "policy"),
        MutationSource::Unknown => ("unknown", "unknown"),
    }
}
