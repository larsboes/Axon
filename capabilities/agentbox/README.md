# agentbox

A coding agent in a disposable micro-VM, talking to a model server on the host. One project
directory in, no network out, nothing left behind on exit.

A modern coding agent reads your files, runs shell commands and installs whatever it decides it
needs. That is what makes it useful, and it is also why running one directly on a work machine
hands it `~/.ssh`, `~/.aws`, every other repo on the disk, and outbound network reach under your
identity. The interesting question is not whether the model is good. It is what exactly this
process can touch, and whether you can show it.

## The pieces, and why each one is there

```
  host (macOS, Apple Silicon)                 apple-container VM
 ┌────────────────────────────┐   hostOnly   ┌──────────────────────────┐
 │  model server              │   network    │  the agent (pi)          │
 │  OpenAI /v1 + tool-calling │◄─────────────┤  git, ripgrep            │
 │  bound 0.0.0.0             │   no other   │  /workspace  ← 1 project │
 │                            │   route out  │  /home/agent/config      │
 └────────────────────────────┘              └──────────────────────────┘
```

- **[pi](https://github.com/earendil-works/pi)** is the agent: an OpenAI-compatible coding CLI
  with `read`/`write`/`edit`/`bash` tools. It is here because it points at *any* endpoint
  speaking chat-completions, which is what makes a local model a first-class option rather than
  a downgrade, and because it ships a real release binary.
- **[apple/container](https://github.com/apple/container)** is the boundary. Each container is
  its own VM, which is a claim you can defend by pointing at virtualization rather than at
  namespace hardening — and `container network create --internal` is what turns "the agent
  shouldn't call out" into "the agent cannot".
- **The model server stays on the host** because MLX needs Metal and the ANE, and a Linux VM
  exposes neither. That constraint is not a nuisance to work around; it is what produces the
  split above.

Verdicts, pins and licences for all three live in `upstreams.toml`, not here.

## The model side

The box needs one thing from the host: an HTTP endpoint speaking OpenAI chat-completions **with
tool-calling**. Without tool-calling the agent is inert — it answers, it never edits, and nothing
in the logs says why. That is the single most common way this setup looks broken while being
correctly configured.

Which server is a config value, not a design decision. [oMLX](https://github.com/jundot/omlx) and LM Studio
are the MLX-native options on Apple Silicon; llama.cpp's server and Ollama's `/v1` speak the same
protocol. Swapping one for another is the overlay's `model_base_url` and nothing else — the
provider inside the box is called `local` precisely so it names the endpoint rather than the
software behind it. Whichever runs has to bind `0.0.0.0` rather than loopback, and therefore has
to enforce an API key: see Setup.

For the model itself, the useful class is a coder-tuned MoE at 4-bit — around 30B total with a
few billion active parameters, roughly 17 GB resident, which is what makes this viable on a
laptop at all. A general instruct model works and is noticeably worse at holding a multi-step
tool loop together. The concrete id, its context window and its token limits are instance
values and live in the overlay's `agentbox.toml`, because the answer differs per machine and per
how much RAM you are willing to give up.

## Verdict

**Adopt pi, contain it with `apple-container`.** pi's own `docs/security.md` says it ships no
sandbox by design and that isolation has to come from the OS or a virtualization boundary, so
this follows the tool's security model rather than bolting one on. Three things follow from
that, and they are the whole design:

1. **The agent binary lives only in an image.** Consumed as the release tarball, sha256-verified
   on the host before the build and again inside it, so the image needs no Node and no npm and
   README.md#language-tooling is satisfied rather than excepted.
2. **The box is host-only and disposable.** Its own `hostOnly` network reaches the model
   endpoint and nothing else; `--rm` discards the VM and its writable layer on exit.
3. **Exactly two things cross the boundary,** both by mount: one project directory at
   `/workspace`, and the agent's config directory. Never the host's own agent config — mounting
   that would hand the container your sessions, settings and credentials, which upstream's
   security doc calls out by name.

Inference stays native on the host. MLX needs Metal and the ANE, which a Linux VM does not
expose, and that constraint is what produces the clean split: model on the host, tool-calling in
the box, one bridge between them.

## Considered and declined

- **Gondolin** ([earendil-works/gondolin](https://github.com/earendil-works/gondolin), pi's own
  micro-VM extension) — keeps pi on the host and routes only its built-in tools into a VM.
  Rejected because it inverts the property this capability exists for: the agent process, its
  extensions and its credentials all stay on the host, and it needs Node ≥ 23.6 plus QEMU
  installed there. The point here is that the host runs the model and nothing else.
- **NVIDIA OpenShell** — policy-controlled sandbox with filesystem, process, credential and
  inference controls, the richest of the three upstream patterns. Rejected for now because every
  sandbox needs a running gateway: a second service to operate and audit for a box that a
  network and two mounts already contain.
- **Docker Desktop** — the reflex answer, and it would work. `apple-container` wins on the one
  sentence a sovereignty review actually needs: each container is its own VM. That is a
  virtualization boundary rather than namespace hardening, and it comes from an Apple-signed
  component with no third-party daemon. `<overlay>/config/machine.toml` already names it as this
  machine's runtime.

  _Trigger that reopens this (README.md#decisions-live-with-their-owner): a Linux/WSL host, where `apple-container` does not
  exist._ There, docker/podman is the available runtime and the boundary is namespaces, not a
  VM — weaker, and the README must say so plainly rather than imply parity. The host-side model
  premise also changes: no Metal/ANE, so inference is a different setup (a Linux GPU/CPU server),
  not MLX-on-host. **Planned: a `docker`/`podman` runtime variant for Linux/WSL** — parked
  pending reference sources from the owner; not built yet. Until then this capability is
  macOS/apple-container only; a Linux deployment currently runs pi natively outside this capability.
- **npm-installing the agent into a `node:22` image**, the shape of the writeup this was built
  from. Declined on two counts: README.md#language-tooling bans npm in Axon code, and the release binary makes the
  entire Node layer unnecessary. The writeup also predates the project's rename, so its package
  (`@mariozechner/pi-coding-agent`) stopped receiving releases at 0.73.1 while pi moved on to
  0.82.x under `@earendil-works`.

## Layout

```
Containerfile                  agent-neutral: base image, git + ripgrep, non-root user, the archive
agentbox                       build / run / shell / doctor
agentbox.toml.example          the shape of <overlay>/config/agentbox.toml
profiles/pi/
  profile.toml                 version, archive url + sha256, config-dir contract, default model args
  AGENTS.md.tmpl               the operating contract, rendered into every session
  models.json.tmpl             provider definition, rendered with the endpoint and key
  extensions/protected-paths/  the seatbelt for the day a mount gets widened
```

Nothing under `profiles/` names a machine, and nothing in the overlay names a version. Bumping
the agent is `version` + `archive_sha256_*` in `profile.toml` and a rebuild; changing the model
or the endpoint is the overlay's `agentbox.toml` and nothing else.

## Setup

1. **The model server bound to `0.0.0.0`**, not only `127.0.0.1`. The box is a separate VM and
   cannot reach the host's loopback; that is the most common reason "it can't reach the model".
   Binding wider also puts the endpoint on whatever network the machine is currently joined to,
   so its API-key check has to stay on — from that moment the key is the access control, and on
   an untrusted network the honest move is to bind back to loopback and let the box stop working.
2. **The key**, in the overlay's gitignored env file. Write it yourself, in your own terminal;
   never through an agent turn (README.md#secrets). `secrets/agentbox-model-key.md` in the overlay has the
   command.
3. **`<overlay>/config/agentbox.toml`** — copy `agentbox.toml.example`, set the model id and
   port.
4. `agentbox build`, then `agentbox doctor`.

Grant macOS Local Network access when it asks. Denied, container-to-host traffic is dropped
silently: no error, no reply, nothing obviously misconfigured. *System Settings → Privacy &
Security → Local Network*.

## Use

```sh
agentbox run --project ~/code/some-repo         # the box, on that project
agentbox run                                    # the current directory
agentbox run --online                           # swap in a network with internet
agentbox run -- --tools read,grep,find,ls -p "Review src/"   # anything after -- goes to the agent
agentbox shell                                  # a prompt in the same box, no agent
```

`--project` is the only host path the box sees, so set it deliberately — unset, it mounts the
current directory. Home and `/` are refused outright.

Files the agent writes into the project are owned by you: the image is built with your uid and
gid, which is why they do not land as a foreign uid 1000.

## What host-install refuses

`agentbox host-install` puts the pinned agent on the host itself, outside the box, plus a shim
that refuses the agent's own `update` verbs.

What it enforces:

| Check | Kind | Source of the rule |
|---|---|---|
| sha256 of the release archive | hard refusal, in `fetch_archive` on the install path | `archive_sha256_${OS}_${ARCH}` in the overlay's `agentbox.toml` |
| published GHSAs against the version | **not checked — yours to do** | see below |

There was a separate `agentbox gate` verb, and a cooldown on how old the release was, until
2026-09-02. Q_AUDIT removed the adoption hold (README.md#patch-first), and with it gone the verb
refused nothing at all — it resolved a repository, printed two lines and returned 0. It is
deleted rather than kept, because a step that cannot say no is how a green line comes to read as
a passed check. What remains is the sha256, which was never in the gate, and the printed
advisory reminder, which `host-install` now prints itself.

**Advisories are a manual step, and host-install prints that every run.** Until 2026-08-28 it
fetched published advisories and refused an install whose target sat inside an affected range.
PRD Q41 retired the script behind that (`tools/lib/advisories.sh`) with the rest of Axon's
homegrown supply-chain plumbing, and no standard tool replaces it *for this dependency*: the
agent arrives as a pinned release tarball, so it appears in no lockfile, which is exactly what
GitHub's Dependabot alerts and `osv-scanner` both read. Dependabot does not watch release
tarballs either, so the tool swap changes nothing here — and a new tag is freshness, not
exposure.

So before you accept a bump, open
[the repository's advisories](https://github.com/earendil-works/pi/security/advisories) and read
them against the version you are about to install. This is a genuine reduction in what the
machine checks for you, stated here rather than discovered later — and host-install refuses to
be silent about it, because a step that prints nothing reads as a step that passed.

## The boundary, and how it was checked

Verified on 2026-07-28, M4 Pro / macOS 26.5.2 / `container` 1.0.0. Same image, same probe, two
networks, so the control shows the block is the network and not a broken image:

| Probe from inside the box | `agentbox` (hostOnly) | `default` (control) |
|---|---|---|
| TCP 1.1.1.1:443 | blocked | reachable |
| DNS lookup | dead | resolves |
| model endpoint on the host gateway | `HTTP/1.1 401` | — |

A 401 is the pass condition there: it proves the endpoint answered.

Then the same five things through a real session, because a blocked socket says nothing about
what the agent does with one:

| Asked of the agent | Result |
|---|---|
| answer a prompt | round-trips the model on the host |
| list and read a file in `/workspace` | reads it, reports the contents |
| edit a file in `/workspace` | lands on the host file, still owned by the invoking user |
| `cat` its own config directory | `Blocked by agentbox: /home/agent is off limits inside the box` |
| `git ls-remote` a GitHub url | `Could not resolve host: github.com`, exit 128, reported honestly |

The fourth is the extension doing its job, not the mount: `/home/agent/config` is mounted and
readable, and holds the model API key. The boundary keeps the host out; the extension keeps the
agent out of its own credentials.

Two findings worth keeping. `container run --no-dns` does not leave the box without a resolver —
the runtime falls back to its built-in 1.1.1.1, which is unreachable on a host-only network, so
every lookup burns its full timeout. Pointing the resolver at the box's own loopback instead
fails in about a millisecond. And the bridge interface for a host-only network exists only while
a container is attached to it, which is why `doctor` probes the model from inside a throwaway
box rather than curling the gateway from the host.

## Troubleshooting

- **The agent answers but never edits.** The model has no tool-calling, or the server does not
  support it. A chat-only model looks like it works and silently does nothing.
- **Requests hang with no error.** macOS Local Network permission, almost always. Grant it, then
  fully quit and reopen the app you are testing from.
- **`doctor` says silence at the endpoint.** The server is bound to loopback only, or it rejects
  the gateway address as a Host header.
- **Startup takes seconds longer than it should.** Something is trying to resolve a name. The
  agent runs with its offline flag set for exactly this reason.
- **A task genuinely needs the network.** `--online`. It is a deliberate, per-run decision, and
  the session's operating contract is rendered to say so.

## Not covered

Multi-project and concurrent sessions, image lifecycle beyond `build`, and supply-chain
attestation for the image itself. Egress filtering is covered, which the writeup this came from
left out of scope.

## Origin

The pattern comes from Michael Hannecke's [*A Sovereign Coding Agent on macOS — PI in an Apple
Container; Zero npm on the
Host*](https://medium.com/@michael.hannecke/a-sovereign-coding-agent-on-macos-pi-in-an-apple-container-zero-npm-on-the-host-46f62ffade0a)
([repo, archived](https://github.com/michaelhannecke/pi-container)). The architecture is his: pi
in an Apple container, model native on the host, config and one project as the only mounts.

Two things in it no longer hold, which is worth knowing before following it directly. The project
was renamed — `badlogic/pi-mono` is now `earendil-works/pi`, and its npm package moved from
`@mariozechner/pi-coding-agent` (last release 0.73.1) to `@earendil-works/`, so the article's
`npm install -g` pulls a version months behind a project that ships a self-contained binary and
needs no Node at all. And its `192.168.64.1` is a default, not an address: read the gateway off
the network, which is what the launcher does.

Everything else that differs is deliberate and argued above: binary over npm, `--internal` over
policy-only egress, host uid over uid 1000, and the API key resolved from the overlay rather than
written into a tracked `models.json`.
