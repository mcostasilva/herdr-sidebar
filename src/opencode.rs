//! OpenCode V2 connection discovery, session forking, and terminal attachment.

use crate::{herdr::Herdr, process};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    env,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Connection {
    pub binary: PathBuf,
    pub server: Option<String>,
}
#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}
#[derive(Deserialize)]
pub struct Session {
    pub id: String,
    pub location: Location,
    pub fork: Option<Fork>,
}
#[derive(Deserialize)]
pub struct Location {
    pub directory: PathBuf,
}
#[derive(Deserialize)]
pub struct Fork {
    #[serde(rename = "sessionID")]
    pub session_id: String,
}

pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        id.starts_with("ses_")
            && id.len() > 4
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid OpenCode session ID"
    );
    Ok(())
}

impl Connection {
    pub fn from_argv(argv: &[String], cwd: Option<&str>) -> Result<Self> {
        let binary = argv.first().context("OpenCode process has no argv")?;
        let mut server = None;
        let mut args = argv.iter().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--" {
                break;
            }
            ensure!(
                arg != "--standalone",
                "standalone OpenCode servers are not supported; use the shared service or --server"
            );
            // Skip values that may themselves look like options.
            if matches!(arg.as_str(), "--prompt" | "--session" | "-s") {
                args.next();
                continue;
            }
            if arg == "--server" {
                server = Some(args.next().context("--server has no URL")?.clone());
            } else if let Some(url) = arg.strip_prefix("--server=") {
                server = Some(url.to_owned());
            }
        }
        if let Some(server) = &server {
            ensure!(
                !server.is_empty() && !server.starts_with('-'),
                "--server has no URL"
            );
        }
        let mut binary = PathBuf::from(binary);
        if binary.is_relative() && binary.components().count() > 1 {
            binary =
                Path::new(cwd.context("relative executable path has no process cwd")?).join(binary);
        }
        Ok(Self { binary, server })
    }

    pub fn discover(herdr: &Herdr<'_>, pane: &str) -> Result<Self> {
        for process in herdr.processes(pane)? {
            if process.argv.first().is_some_and(|arg| {
                Path::new(arg)
                    .file_name()
                    .is_some_and(|name| name == "opencode")
            }) {
                return Self::from_argv(&process.argv, process.cwd.as_deref());
            }
        }
        bail!("could not identify the source OpenCode process and server")
    }

    fn api<T: DeserializeOwned>(
        &self,
        cwd: &Path,
        method: &str,
        path: &str,
        fork: bool,
        timeout: Duration,
    ) -> Result<T> {
        let mut command = Command::new(&self.binary);
        command.current_dir(cwd).arg("api");
        if let Some(server) = &self.server {
            command.args(["--server", server]);
        }
        command.args([method, path]);
        // V2.0.11: {} copies the full projected history.
        if fork {
            command.args(["--data", "{}"]);
        }
        let response: Envelope<T> = process::json(&mut command, timeout)?;
        Ok(response.data)
    }

    pub fn get(&self, id: &str, cwd: &Path, timeout: Duration) -> Result<Session> {
        validate_id(id)?;
        let session: Session =
            self.api(cwd, "get", &format!("/api/session/{id}"), false, timeout)?;
        ensure!(session.id == id, "OpenCode returned a different session");
        Ok(session)
    }

    pub fn fork(&self, id: &str, cwd: &Path, timeout: Duration) -> Result<Session> {
        validate_id(id)?;
        let session: Session = self.api(
            cwd,
            "post",
            &format!("/api/session/{id}/fork"),
            true,
            timeout,
        )?;
        validate_id(&session.id)?;
        ensure!(
            session.id != id
                && session
                    .fork
                    .as_ref()
                    .is_some_and(|fork| fork.session_id == id),
            "OpenCode did not return a fork of {id}"
        );
        Ok(session)
    }

    pub fn ensure_forkable(&self, id: &str, cwd: &Path, timeout: Duration) -> Result<()> {
        validate_id(id)?;
        let messages: Vec<serde::de::IgnoredAny> = self.api(
            cwd,
            "get",
            &format!("/api/session/{id}/message?limit=1"),
            false,
            timeout,
        )?;
        ensure!(
            !messages.is_empty(),
            "this conversation is empty; OpenCode needs saved history before it can be forked"
        );
        Ok(())
    }
}

pub fn attach() -> Result<()> {
    let session = env::var("HERDR_SIDEBAR_SESSION").context("missing fork session ID")?;
    validate_id(&session)?;
    let binary = env::var_os("HERDR_SIDEBAR_BIN").context("missing OpenCode executable")?;
    let mut command = Command::new(binary);
    if let Ok(server) = env::var("HERDR_SIDEBAR_SERVER")
        && !server.is_empty()
    {
        command.args(["--server", &server]);
    }
    command.args(["--session", &session]);
    // Preserve Herdr's PTY; its integration should see OpenCode as foreground.
    Err(command.exec()).context("launch the forked OpenCode session")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn connection_options_and_literal_prompt_values() {
        for args in [
            argv(&["opencode", "--server", "http://localhost:4096"]),
            argv(&["opencode", "--server=http://localhost:4096"]),
        ] {
            assert_eq!(
                Connection::from_argv(&args, None)
                    .unwrap()
                    .server
                    .as_deref(),
                Some("http://localhost:4096")
            );
        }
        assert!(Connection::from_argv(&argv(&["opencode", "--standalone"]), None).is_err());
        assert!(
            Connection::from_argv(&argv(&["opencode", "--server", "--session"]), None).is_err()
        );
        assert!(
            Connection::from_argv(&argv(&["opencode", "--prompt", "--standalone"]), None).is_ok()
        );
        assert_eq!(
            Connection::from_argv(&argv(&["./bin/opencode"]), Some("/project"))
                .unwrap()
                .binary,
            Path::new("/project/./bin/opencode")
        );
    }
}
