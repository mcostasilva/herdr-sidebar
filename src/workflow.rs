//! Fork-or-focus workflow. Persist intent before remote side effects.

use crate::{
    Config, Options,
    herdr::{Herdr, Pane, PaneRef},
    opencode::Connection,
    state::{Link, Pending, Stage, Store},
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct Outcome {
    action: &'static str,
    pane_id: String,
    session_id: String,
}

fn matching(panes: &[Pane], session: &str) -> Result<Option<PaneRef>> {
    let candidates: Vec<_> = panes
        .iter()
        .filter(|pane| pane.session() == Some(session))
        .collect();
    ensure!(
        candidates.len() <= 1,
        "multiple panes display {session}; close the extra attachment before retrying"
    );
    Ok(candidates.first().map(|pane| pane.identity.clone()))
}

pub fn open(config: &Config, options: &Options) -> Result<Outcome> {
    let herdr = Herdr(config);
    let mut store = Store::acquire(&config.state_dir, config.timeout)?;
    let panes = herdr.panes()?;
    let source = panes
        .iter()
        .find(|pane| pane.identity.pane_id == config.pane)
        .context("originating pane no longer exists")?;
    let live: Vec<_> = panes.iter().map(|pane| pane.identity.clone()).collect();
    if let Some(link) = store
        .state
        .existing(&config.socket, &source.identity, &live)
    {
        herdr.focus(&link.side.pane_id)?;
        store.save()?;
        return Ok(Outcome {
            action: "focused",
            pane_id: link.side.pane_id,
            session_id: link.session_id,
        });
    }
    let index = if let Some(index) = store
        .state
        .pending
        .iter()
        .position(|p| p.socket == config.socket && p.source == source.identity)
    {
        index
    } else {
        ensure!(
            options.recover_session.is_none() && !options.retry_launch && !options.retry_fork,
            "no interrupted operation to recover"
        );
        ensure!(
            source.agent.as_deref() == Some("opencode"),
            "source pane is not running OpenCode"
        );
        let parent_id = source
            .session()
            .context("Herdr has no OpenCode session ID for this pane")?;
        let connection = Connection::discover(&herdr, &config.pane)?;
        let parent = connection.get(parent_id, Path::new("."), config.timeout)?;
        connection.ensure_forkable(parent_id, &parent.location.directory, config.timeout)?;
        let index = store.state.pending.len();
        store.state.pending.push(Pending {
            socket: config.socket.clone(),
            source: source.identity.clone(),
            parent_id: parent_id.into(),
            connection: connection.clone(),
            cwd: parent.location.directory.clone(),
            stage: Stage::Forking,
        });
        store.save()?;
        let fork = connection.fork(parent_id, &parent.location.directory, config.timeout).context(
            "fork outcome could not be confirmed; it will not be repeated automatically. Inspect OpenCode sessions and use open --recover-session ID once the request has settled"
        )?;
        store.state.pending[index].stage = Stage::Forked {
            session_id: fork.id.clone(),
        };
        store.save().with_context(|| {
            format!(
                "fork {} exists; recover it with open --recover-session {}",
                fork.id, fork.id
            )
        })?;
        index
    };

    ensure!(
        !(options.retry_fork && options.recover_session.is_some()),
        "choose either --retry-fork or --recover-session"
    );
    if options.retry_fork {
        let pending = &store.state.pending[index];
        ensure!(
            matches!(pending.stage, Stage::Forking),
            "fork ID is already recorded; retry the normal action"
        );
        let fork = pending
            .connection
            .fork(&pending.parent_id, &pending.cwd, config.timeout)
            .context("fork outcome could not be confirmed; inspect OpenCode before retrying")?;
        store.state.pending[index].stage = Stage::Forked {
            session_id: fork.id,
        };
        store.save()?;
    }
    if let Some(id) = &options.recover_session {
        let pending = &store.state.pending[index];
        ensure!(
            matches!(pending.stage, Stage::Forking),
            "fork ID is already recorded; retry the normal action"
        );
        let session = pending.connection.get(id, &pending.cwd, config.timeout)?;
        ensure!(
            session.id != pending.parent_id
                && session
                    .fork
                    .as_ref()
                    .is_some_and(|fork| fork.session_id == pending.parent_id),
            "recovery session is not a fork of {}",
            pending.parent_id
        );
        ensure!(
            session.location.directory == pending.cwd,
            "recovery session uses a different working directory"
        );
        store.state.pending[index].stage = Stage::Forked {
            session_id: id.clone(),
        };
        store.save()?;
    }

    let pending = store.state.pending[index].clone();
    let (session, mut side, uncertain) = match &pending.stage {
        Stage::Forking => bail!(
            "previous fork outcome is unknown; inspect OpenCode sessions and use open --recover-session ID, or --retry-fork after verifying no fork was created. No new fork was created"
        ),
        Stage::Forked { session_id } => (session_id.clone(), matching(&panes, session_id)?, false),
        Stage::Opening { session_id, side } => {
            let existing = side
                .as_ref()
                .filter(|side| live.contains(side))
                .cloned()
                .or(matching(&panes, session_id)?);
            (session_id.clone(), existing, side.is_none())
        }
    };
    if side.is_none() {
        ensure!(
            !uncertain || options.retry_launch,
            "previous pane launch outcome is unknown. Let Herdr settle, then retry; if no pane appeared, use open --retry-launch to reopen the saved fork {session}"
        );
        store.state.pending[index].stage = Stage::Opening {
            session_id: session.clone(),
            side: None,
        };
        store.save()?;
        side = Some(herdr.open(&config.pane, &pending.cwd, &session, &pending.connection)
            .with_context(|| format!("fork {session} is saved; retry the action to reconcile its pane, or use open --retry-launch after checking the layout"))?);
    }
    let side = side.context("missing side pane")?;
    store.state.pending[index].stage = Stage::Opening {
        session_id: session.clone(),
        side: Some(side.clone()),
    };
    store.save()?;
    herdr.wait_ready(&side, &session).with_context(|| {
        format!(
            "pane {} did not attach to {session}; retry once startup settles",
            side.pane_id
        )
    })?;
    store.state.links.push(Link {
        socket: config.socket.clone(),
        source: pending.source,
        side: side.clone(),
        session_id: session.clone(),
    });
    store.state.pending.remove(index);
    store.save()?;
    herdr.focus(&side.pane_id)?;
    Ok(Outcome {
        action: "created",
        pane_id: side.pane_id,
        session_id: session,
    })
}
