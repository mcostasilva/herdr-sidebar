# Changelog

## 0.2.0

- Rename the project, repository, Rust binary, and plugin ID to `herdr-sidebar`.
- Rename the action to `herdr-sidebar.open` and use agent-neutral plugin and pane titles.
- Use the `HERDR_SIDEBAR_*` environment prefix; keep `OPENCODE_SIDEBAR_TIMEOUT_MS` as a fallback for existing configurations.
- Document migration from `opencode-sidebar`, including preserving saved pane mappings.
- OpenCode V2 remains the currently supported agent.

## 0.1.0

- Fork the current OpenCode V2 conversation into a right-hand Herdr split.
- Reuse and focus that side pane on subsequent shortcut invocations.
- Preserve the conversation, agent/model, working directory, and explicit server URL.
- Verify that the new pane attaches to the expected fork session.
- Explain empty conversations before attempting a fork.
- Serialize concurrent invocations and bound command, lock, and startup waits.
- Recover saved forks and reconcile interrupted pane launches.
- Support macOS and Linux, with MIT licensing and source-build installation.
