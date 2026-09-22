//! Durable mappings and in-progress operations, serialized by a bounded file lock.

use crate::{herdr::PaneRef, opencode::Connection, process::POLL};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Link {
    pub socket: String,
    pub source: PaneRef,
    pub side: PaneRef,
    pub session_id: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Pending {
    pub socket: String,
    pub source: PaneRef,
    pub parent_id: String,
    pub connection: Connection,
    pub cwd: PathBuf,
    pub stage: Stage,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stage {
    /// Written before the request: a lost response must not cause another fork.
    Forking,
    Forked {
        session_id: String,
    },
    /// Written before pane creation; the response may be lost after creation.
    Opening {
        session_id: String,
        side: Option<PaneRef>,
    },
}

fn legacy_version() -> u32 {
    1
}

#[derive(Deserialize, Serialize)]
pub struct State {
    #[serde(default = "legacy_version")]
    version: u32,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub pending: Vec<Pending>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: 2,
            links: vec![],
            pending: vec![],
        }
    }
}

impl State {
    pub fn existing(&mut self, socket: &str, current: &PaneRef, live: &[PaneRef]) -> Option<Link> {
        self.links
            .retain(|link| link.socket != socket || live.contains(&link.side));
        // Side-first lookup also works after the original source was closed.
        self.links
            .iter()
            .find(|link| link.socket == socket && &link.side == current)
            .or_else(|| {
                self.links
                    .iter()
                    .find(|link| link.socket == socket && &link.source == current)
            })
            .cloned()
    }
}

pub struct Store {
    _lock: File,
    directory: PathBuf,
    pub state: State,
}

impl Store {
    pub fn acquire(directory: &Path, timeout: Duration) -> Result<Self> {
        fs::create_dir_all(directory).context("create sidebar state directory")?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("state.lock"))
            .context("open sidebar state lock")?;
        let start = Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    ensure!(
                        start.elapsed() < timeout,
                        "another sidebar action is still running; retry shortly"
                    );
                    thread::sleep(POLL);
                }
                Err(error) => return Err(error.into()),
            }
        }
        let mut state: State = match fs::read(directory.join("state.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("invalid sidebar state.json; preserve it for recovery")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(error) => return Err(error).context("read sidebar mappings"),
        };
        ensure!(
            (1..=2).contains(&state.version),
            "unsupported sidebar state version {}",
            state.version
        );
        state.version = 2;
        Ok(Self {
            _lock: lock,
            directory: directory.to_owned(),
            state,
        })
    }

    pub fn save(&self) -> Result<()> {
        // The separate lock stays held while state.json is atomically replaced.
        let mut temp = File::create(self.directory.join("state.tmp"))?;
        temp.write_all(&serde_json::to_vec_pretty(&self.state)?)?;
        temp.sync_all()?;
        fs::rename(
            self.directory.join("state.tmp"),
            self.directory.join("state.json"),
        )?;
        File::open(&self.directory)?
            .sync_all()
            .context("persist sidebar mappings")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pane(id: &str, terminal: &str) -> PaneRef {
        PaneRef {
            pane_id: id.into(),
            terminal_id: terminal.into(),
        }
    }
    fn linked() -> (State, PaneRef, PaneRef) {
        let source = pane("w1:p1", "source");
        let side = pane("w1:p2", "side");
        let state = State {
            links: vec![Link {
                socket: "a".into(),
                source: source.clone(),
                side: side.clone(),
                session_id: "ses_fork".into(),
            }],
            ..State::default()
        };
        (state, source, side)
    }
    #[test]
    fn source_and_orphaned_side_reuse_the_same_mapping() {
        let (mut state, source, side) = linked();
        assert_eq!(
            state
                .existing("a", &source, &[source.clone(), side.clone()])
                .unwrap()
                .side,
            side
        );
        assert_eq!(
            state
                .existing("a", &side, std::slice::from_ref(&side))
                .unwrap()
                .side,
            side
        );
    }
    #[test]
    fn recycled_ids_and_other_sockets_cannot_reuse_a_mapping() {
        let (mut state, source, side) = linked();
        assert!(
            state
                .existing("b", &source, &[source.clone(), side.clone()])
                .is_none()
        );
        assert_eq!(state.links.len(), 1);
        let reused = pane(&side.pane_id, "different-terminal");
        assert!(
            state
                .existing("a", &source, &[source.clone(), reused])
                .is_none()
        );
        assert!(state.links.is_empty());
    }
    #[test]
    fn legacy_mapping_survives_upgrade() {
        let (state, source, side) = linked();
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("state.json"),
            serde_json::to_vec(&serde_json::json!({"links": state.links})).unwrap(),
        )
        .unwrap();
        let mut store = Store::acquire(directory.path(), Duration::from_secs(1)).unwrap();
        assert!(
            store
                .state
                .existing("a", &source, &[source.clone(), side])
                .is_some()
        );
        store.save().unwrap();
    }
}
