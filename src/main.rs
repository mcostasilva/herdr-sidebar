//! A Herdr action that forks or focuses an OpenCode conversation.

mod herdr;
mod opencode;
mod process;
mod state;
mod workflow;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode, time::Duration};

const PLUGIN: &str = "herdr-sidebar";

/// Configuration is captured once; workflow code never changes global environment.
struct Config {
    herdr: OsString,
    socket: String,
    state_dir: PathBuf,
    pane: String,
    timeout: Duration,
}

#[derive(Default)]
struct Options {
    pane: Option<String>,
    recover_session: Option<String>,
    retry_launch: bool,
    retry_fork: bool,
}

enum Cli {
    Help,
    Version,
    Attach,
    Open(Options),
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Cli> {
    let mut args = args.peekable();
    let Some(command) = args.next() else {
        return Ok(Cli::Help);
    };
    let simple = match command.to_str() {
        Some("--help" | "-h") => Some(Cli::Help),
        Some("--version" | "-V") => Some(Cli::Version),
        Some("attach") => Some(Cli::Attach),
        Some("open") => None,
        _ => bail!("unknown command; use --help"),
    };
    if let Some(command) = simple {
        ensure!(args.next().is_none(), "unexpected argument; use --help");
        return Ok(command);
    }
    let mut options = Options::default();
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--pane" | "--recover-session") => {
                let value = args
                    .next()
                    .context("option requires a value")?
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("IDs must be UTF-8"))?;
                ensure!(
                    !value.starts_with('-') && !value.is_empty(),
                    "option requires a value"
                );
                if flag == "--pane" {
                    options.pane = Some(value)
                } else {
                    options.recover_session = Some(value)
                }
            }
            Some("--retry-launch") => options.retry_launch = true,
            Some("--retry-fork") => options.retry_fork = true,
            Some("--help" | "-h") => return Ok(Cli::Help),
            _ => bail!("unknown option {}; use --help", flag.to_string_lossy()),
        }
    }
    Ok(Cli::Open(options))
}

impl Config {
    fn from_env(options: &Options) -> Result<Self> {
        #[derive(Deserialize)]
        struct Invocation {
            focused_pane_id: String,
        }

        let pane = if let Some(pane) = &options.pane {
            pane.clone()
        } else if let Ok(context) = env::var("HERDR_PLUGIN_CONTEXT_JSON") {
            // Prefer the invocation snapshot over the UI's current focus.
            serde_json::from_str::<Invocation>(&context)
                .context("invalid Herdr invocation context")?
                .focused_pane_id
        } else {
            env::var("HERDR_PANE_ID").context("missing source pane; use --pane PANE_ID")?
        };
        let timeout = match env::var("HERDR_SIDEBAR_TIMEOUT_MS").or_else(|error| match error {
            env::VarError::NotPresent => env::var("OPENCODE_SIDEBAR_TIMEOUT_MS"),
            error => Err(error),
        }) {
            Ok(value) => value
                .parse::<u64>()
                .context("invalid sidebar timeout (HERDR_SIDEBAR_TIMEOUT_MS or legacy OPENCODE_SIDEBAR_TIMEOUT_MS)")?,
            Err(env::VarError::NotPresent) => 30_000,
            Err(error) => return Err(error.into()),
        };
        ensure!(timeout > 0, "timeout must be greater than zero");
        Ok(Self {
            herdr: env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()),
            socket: env::var("HERDR_SOCKET_PATH").context("HERDR_SOCKET_PATH is missing")?,
            state_dir: env::var_os("HERDR_PLUGIN_STATE_DIR")
                .context("invoke the Herdr action, or set HERDR_PLUGIN_STATE_DIR for direct use")?
                .into(),
            pane,
            timeout: Duration::from_millis(timeout),
        })
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli {
        Cli::Help => println!(
            "Herdr Sidebar\n\n\
            Usage: herdr-sidebar open [OPTIONS]\n\n\
            Fork the current conversation into a right-hand pane, or focus its existing fork.\n\n\
            Options:\n  --pane PANE_ID           Override the originating pane\n  \
            --recover-session ID     Recover an interrupted fork using its known session ID\n  \
            --retry-launch           Retry a launch whose outcome could not be determined\n  \
            --retry-fork             Retry after verifying no fork was created\n  \
            -h, --help               Show help\n  -V, --version            Show version\n\n\
            Herdr action: herdr-sidebar.open"
        ),
        Cli::Version => println!("herdr-sidebar {}", env!("CARGO_PKG_VERSION")),
        Cli::Attach => {
            ensure!(
                env::var("HERDR_ENV").as_deref() == Ok("1"),
                "run inside Herdr"
            );
            opencode::attach()?;
        }
        Cli::Open(options) => {
            ensure!(
                env::var("HERDR_ENV").as_deref() == Ok("1"),
                "run inside Herdr"
            );
            let config = Config::from_env(&options)?;
            println!(
                "{}",
                serde_json::to_string(&workflow::open(&config, &options)?)?
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = match parse(env::args_os().skip(1)) {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("Herdr Sidebar: {error:#}");
            return ExitCode::from(2);
        }
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Herdr Sidebar: {error:#}");
            ExitCode::FAILURE
        }
    }
}
