# Contributing

## Setup

Install a stable Rust toolchain. The minimum supported Rust version is **1.89**.
Python 3.11+ is used only for the release-metadata check.

```sh
git clone https://github.com/mcostasilva/opencode-herdr-sidebar.git
cd opencode-herdr-sidebar
cargo build --release --locked --target-dir target
```

## Checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 scripts/check-release.py
```

To verify the minimum Rust version:

```sh
rustup toolchain install 1.89.0 --profile minimal
cargo +1.89.0 test --locked
```

Integration tests launch the real plugin binary against temporary shell fixtures.
They do not require Herdr, OpenCode, a provider account, or a running agent. Each
test has its own environment and state directory. Keep new tests focused on
observable workflow behavior and failure recovery.

## Structure

| Module | Responsibility |
| --- | --- |
| `main.rs` | CLI parsing, environment configuration, output and exit status |
| `workflow.rs` | Fork-or-focus orchestration and recovery |
| `herdr.rs` | Typed Herdr responses, pane operations, startup verification |
| `opencode.rs` | Connection discovery, V2 session operations, terminal exec |
| `state.rs` | Versioned mappings, pending operations, locking, atomic persistence |
| `process.rs` | Bounded subprocess execution and output capture |

Use default rustfmt formatting and contextual errors. Preserve the exact source
session identity and server connection. The state lock spans remote operations;
every wait must remain bounded. Record intent before creating sessions or panes,
and do not automatically repeat an operation whose outcome is unknown.

## Live development

```sh
herdr plugin link "$PWD"
herdr plugin action invoke opencode-sidebar.open
herdr plugin log list --plugin opencode-sidebar --limit 5
```

Invoke the action with the intended source pane focused. Rebuild after changes;
the next action loads the new binary. For isolation, use a disposable Herdr session
and an OpenCode conversation created specifically for testing.

Check first-use creation, repeated focus, invocation from inside the fork, closing
and reopening, and agent/model/history inheritance. Confirm the source conversation
is unaffected. Avoid sending prompts during a basic launch test.

## Releases

1. Update `Cargo.toml`, `Cargo.lock`, `herdr-plugin.toml`, and `CHANGELOG.md` together.
2. Run the checks and the live workflow smoke test.
3. Commit and push, then tag the commit `vX.Y.Z` and push that tag.

The release workflow runs CI on macOS/Linux and Rust 1.89 before publishing a
GitHub Release. Users install the source package through Herdr; Cargo builds the
binary into the exact directory declared by the manifest. Keep `Cargo.lock` in Git.

## License

Contributions are licensed under the project's [MIT license](LICENSE).
