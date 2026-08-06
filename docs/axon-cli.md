# Axon CLI

`axon` is the public command interface for operating an Axon installation. It is available to
humans and agents independently of any installed skill. Every command resolves the current Axon
checkout from its own location, so a moved checkout does not change its interface.

## Contract

`axon help` is the discovery entrypoint. `axon search <words...>` narrows the command and current
capability/Pack inventory without requiring an agent to read repository files first.

| Command | Owns |
| --- | --- |
| `axon capability list\|health\|url\|call` | Live capability registry and HTTP surfaces |
| `axon capability ingest\|feed` | Common `comms` operations |
| `axon pack list` | Available public and active-overlay Packs |
| `axon pack <status\|deploy\|sync\|remove> <harness> ...` | Harness-specific Pack deployment |
| `axon doctor` | Installation health |
| `axon context with\|on ...` | Bounded live operating or repository context |

The harness names are `claude`, `codex`, `opencode`, and `pi`. A command rejects an unknown
harness rather than guessing a destination. The command output is designed for terminal use; the
underlying harness tools retain their own JSON interfaces where they already provide one.

## Installation

`tools/install.sh` creates `~/.local/bin/axon` as a symlink to Axon's tracked launcher. It never
overwrites a non-Axon command at that path. The installer reports the exact shell-path action when
`~/.local/bin` is not on `PATH`.

## Agent Baseline

Harness configuration managed by Axon should inject only this durable discovery instruction:

```text
For Axon operations, run `axon help` or `axon search <task>` before browsing files.
```

This makes the CLI discoverable without loading an Axon-specific skill. Repository policy lives in
the Axon checkout's `AGENTS.md`; operational details live in command help and the relevant Pack or
capability contract.
