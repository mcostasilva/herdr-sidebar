# Changelog

## 0.1.0

- Fork the current OpenCode V2 conversation into a right-hand Herdr split.
- Reuse and focus that side pane on subsequent shortcut invocations.
- Preserve the conversation, agent/model, working directory, and explicit server URL.
- Verify that the new pane attaches to the expected fork session.
- Explain empty conversations before attempting a fork.
- Serialize concurrent invocations and bound command, lock, and startup waits.
- Recover saved forks and reconcile interrupted pane launches.
- Support macOS and Linux, with MIT licensing and source-build installation.
