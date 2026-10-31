//! `knect-e2e` entrypoint.

use std::path::PathBuf;

use clap::Parser;
use knect_e2e::{E2eError, Harness, SCENARIOS, run_all, run_scenario};

#[derive(Debug, Parser)]
#[command(name = "knect-e2e", version, about = "knect end-to-end harness")]
struct Args {
    /// Run every registered scenario.
    #[arg(long)]
    all: bool,
    /// Run one scenario by name.
    #[arg(long, value_name = "NAME")]
    scenario: Option<String>,
    /// Daemon HTTP base URL.
    #[arg(long)]
    daemon_url: Option<String>,
    /// Daemon Unix socket path.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Fixture directory hint for a daemon spawned by the harness.
    #[arg(long)]
    fixture: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), E2eError> {
    let args = Args::parse();
    if args.all && args.scenario.is_some() {
        eprintln!("--all and --scenario are mutually exclusive");
        std::process::exit(2);
    }
    let harness = Harness::new(args.daemon_url.as_deref(), args.socket.as_deref())?;
    if let Some(fixture) = &args.fixture {
        println!("fixture hint: {}", fixture.display());
    }

    match args.scenario {
        Some(name) => {
            println!("running scenario {name}");
            run_scenario(&name, &harness).await?;
            println!("scenario {name} passed (skeleton)");
        }
        None => {
            run_all(&harness).await?;
            println!("all {} scenarios passed (skeleton)", SCENARIOS.len());
        }
    }
    Ok(())
}
