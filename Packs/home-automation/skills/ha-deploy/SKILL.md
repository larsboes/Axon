---
name: ha-deploy
description: Validates, reloads, and verifies an existing Home Assistant deployment through its API. Use when a deployment's configuration has already been placed on the owning host and needs validating, reloading or verifying. Do not use for placing that configuration (use homectl) or for managing the container and its files, which stay outside this skill. Do not use for querying entity state (use ha-cli).
allowed-tools: Bash
---

# ha-deploy

This skill covers the safe API boundary after configuration is already present on the Home
Assistant host:

```sh
scripts/ha-deploy check
scripts/ha-deploy reload
scripts/ha-deploy verify
```

- `check` asks Home Assistant to validate its current configuration.
- `reload` applies reloadable YAML domains without restarting the service.
- `verify` requires Home Assistant to report `RUNNING` and summarizes automation states.

The retired SSH/git-pull deployment path is intentionally absent. Container changes go through
`tools/service-runner.sh`; overlay configuration placement belongs to the owning overlay.

Connection facts come from the active overlay. The token comes from the private secret mechanism
at runtime and is never printed. Confirm before `reload`, because it mutates the running instance.
