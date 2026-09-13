# jcode

A personal, macOS-only fork of [jcode](https://github.com/1jehuang/jcode).

Builds from source only — no prebuilt releases, no package-manager channel. See
[docs/FORK_WORKFLOW.md](docs/FORK_WORKFLOW.md) for what this fork removes and how
it tracks upstream.

## Install

```bash
git clone https://github.com/indrajeetadityaroy9/jcode.git
cd jcode
cargo build --release
scripts/install_release.sh
```

`scripts/install_release.sh` stores the binary under
`~/.jcode/builds/versions/<version>/` and repoints the `~/.local/bin/jcode`
launcher at it. `~/.local/bin` must precede `~/.cargo/bin` on `PATH`.

## Quick start

```bash
jcode                              # TUI
jcode run "say hello"              # one-shot, non-interactive
jcode --resume fox                 # resume by memorable name or session id
jcode serve                        # persistent background daemon
jcode connect                      # attach another client to it
jcode transcript "run the tests"   # inject text into the active TUI
```

## Providers

`jcode login` with no arguments lists the built-in flows interactively;
`jcode login --provider <id>` picks one directly. `jcode auth-test
--all-configured` reports what is already usable without prompting. `/account`
switches between accounts of the same provider.

Credential resolution order, OAuth details, and per-provider environment
variables are documented in [OAUTH.md](OAUTH.md).

## Configuration

`~/.jcode/config.toml` holds everything; every key is optional and the generated
file documents its own defaults. Sessions, auth, logs, and memory also live under
`~/.jcode/`, relocatable with `$JCODE_HOME`.

Fork-specific behavior worth knowing:

- **Web search** queries a SearXNG instance and nothing else. `[websearch] url`
  defaults to `http://127.0.0.1:8080`; `SEARXNG_URL` overrides it. The instance
  needs the JSON format enabled (`formats: [html, json]`).
- **Swarm workers** inherit the coordinator's model unless a spawn passes `model`
  explicitly. `[agents] swarm_model` pins every worker instead; `"inherit"` is
  the default behavior. Routing guidance for spawns is read from
  `./.jcode/swarm-prompt.md`, then `~/.jcode/swarm-prompt.md`, then the built-in
  `crates/jcode-base/src/prompt/swarm_prompt.md`.

## Development

`jcode run` and interactive sessions are served by the long-lived daemon, so a
freshly built binary is inert until the launcher is repointed. To exercise a
build without disturbing the installed daemon, give it its own runtime directory:

```bash
cargo build --profile selfdev
JCODE_RUNTIME_DIR=/tmp/jcode-dev ./target/selfdev/jcode --no-update run 'say hello'
```

[AGENTS.md](AGENTS.md) covers the rest of the runtime-verification procedure and
the repo conventions.

## Uninstall

```bash
scripts/uninstall.sh --yes           # keep config, auth, and sessions
scripts/uninstall.sh --purge --yes   # wipe config, auth, sessions, logs, memory
```

`--dry-run` previews the removals without deleting anything.

## Platform

macOS on Apple Silicon and Intel is the only supported platform; all non-macOS
code paths are deleted from this fork (see
[docs/FORK_WORKFLOW.md](docs/FORK_WORKFLOW.md) §1).

## Further reading

- [Fork Maintenance Runbook](docs/FORK_WORKFLOW.md)
- [Provider and OAuth reference](OAUTH.md)
- [Server Architecture](docs/SERVER_ARCHITECTURE.md)
- [Swarm Architecture](docs/SWARM_ARCHITECTURE.md)
- [Memory Architecture](docs/MEMORY_ARCHITECTURE.md)
- [Ambient Mode](docs/AMBIENT_MODE.md)
- [Wrappers and Shell Integration](docs/WRAPPERS.md)
