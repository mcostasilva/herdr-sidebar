//! Typed views of the Herdr responses used by this plugin.

use crate::{Config, PLUGIN, opencode::Connection, process};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{ffi::OsString, path::Path, process::Command, thread, time::Instant};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PaneRef {
    pub pane_id: String,
    pub terminal_id: String,
}

#[derive(Clone, Deserialize)]
pub struct Pane {
    #[serde(flatten)]
    pub identity: PaneRef,
    pub agent: Option<String>,
    pub agent_session: Option<AgentSession>,
}

#[derive(Clone, Deserialize)]
pub struct AgentSession {
    pub agent: String,
    pub kind: String,
    pub value: String,
}

impl Pane {
    pub fn session(&self) -> Option<&str> {
        self.agent_session
            .as_ref()
            .filter(|s| s.agent == "opencode" && s.kind == "id")
            .map(|s| s.value.as_str())
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    result: T,
}
#[derive(Deserialize)]
struct PaneList {
    panes: Vec<Pane>,
}
#[derive(Deserialize)]
struct Opened {
    plugin_pane: PluginPane,
}
#[derive(Deserialize)]
struct PluginPane {
    pane: Pane,
}
#[derive(Deserialize)]
struct ProcessInfo {
    process_info: Processes,
}
#[derive(Deserialize)]
pub struct Processes {
    pub foreground_processes: Vec<Foreground>,
}
#[derive(Deserialize)]
pub struct Foreground {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

pub struct Herdr<'a>(pub &'a Config);

impl Herdr<'_> {
    fn call<T: DeserializeOwned>(
        &self,
        args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
    ) -> Result<T> {
        let response: Envelope<T> =
            process::json(Command::new(&self.0.herdr).args(args), self.0.timeout)?;
        Ok(response.result)
    }

    pub fn panes(&self) -> Result<Vec<Pane>> {
        Ok(self.call::<PaneList>(["pane", "list"])?.panes)
    }

    pub fn processes(&self, pane: &str) -> Result<Vec<Foreground>> {
        Ok(self
            .call::<ProcessInfo>(["pane", "process-info", "--pane", pane])?
            .process_info
            .foreground_processes)
    }

    pub fn focus(&self, pane: &str) -> Result<()> {
        self.call::<serde_json::Value>(["plugin", "pane", "focus", pane])?;
        Ok(())
    }

    pub fn open(
        &self,
        source: &str,
        cwd: &Path,
        session: &str,
        connection: &Connection,
    ) -> Result<PaneRef> {
        let mut args: Vec<OsString> = [
            "plugin",
            "pane",
            "open",
            "--plugin",
            PLUGIN,
            "--entrypoint",
            "session",
            "--placement",
            "split",
            "--direction",
            "right",
            "--target-pane",
            source,
            "--cwd",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        args.push(cwd.as_os_str().to_owned());
        args.push("--focus".into());
        for (key, value) in [
            ("HERDR_SIDEBAR_SESSION", session),
            (
                "HERDR_SIDEBAR_SERVER",
                connection.server.as_deref().unwrap_or(""),
            ),
            (
                "HERDR_SIDEBAR_BIN",
                connection
                    .binary
                    .to_str()
                    .context("executable path is not UTF-8")?,
            ),
        ] {
            args.extend(["--env".into(), format!("{key}={value}").into()]);
        }
        Ok(self.call::<Opened>(args)?.plugin_pane.pane.identity)
    }

    pub fn wait_ready(&self, pane: &PaneRef, session: &str) -> Result<()> {
        let deadline = Instant::now() + self.0.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(
                !remaining.is_zero(),
                "pane {} did not attach to {session} in time; retry the action after startup",
                pane.pane_id
            );
            // Include each CLI call in the overall startup budget.
            let response: Envelope<PaneList> = process::json(
                Command::new(&self.0.herdr).args(["pane", "list"]),
                remaining,
            )?;
            let live = response
                .result
                .panes
                .into_iter()
                .find(|p| p.identity == *pane)
                .context(
                    "side pane closed before OpenCode attached; retry to reopen the saved fork",
                )?;
            if live.agent.as_deref() == Some("opencode") && live.session() == Some(session) {
                return Ok(());
            }
            thread::sleep(process::POLL);
        }
    }
}
