---
name: red-team
description: Attacks one proposal. Decomposes it into atomic claims, states the steelman its author would sign, then argues the strongest surviving counter-argument, with findings ranked by severity and a test that would settle each one. Use when the request asks to red-team, stress-test, poke holes in, break, challenge, play devil's advocate against, or find the weaknesses of a plan, an argument, a design or a decision that has already been made. Do not use for weighing two or more options against each other (use council), and do not use for scanning software, a network or a host for vulnerabilities.
license: MIT
---

# red-team

One proposal goes in. The strongest honest case for it and the strongest surviving case against
it come out, with each objection ranked and testable.

Red-team attacks a single proposal. To weigh options against each other, use `council`.

Adapted from the RedTeam skill in LifeOS by Daniel Miessler
(https://github.com/danielmiessler/LifeOS), MIT.

## Procedure

Copy this checklist and track progress:

```
- [ ] 1. Restate the proposal as atomic claims
- [ ] 2. Write the steelman
- [ ] 3. Attack every claim through the lenses
- [ ] 4. Rank the findings by severity
- [ ] 5. Name the one core issue
- [ ] 6. Report, and hand off if the answer is a choice
```

**1 — Atomic claims.** Read `references/method.md` and split the proposal into claims that are
each independently true or false. A claim that cannot be false is a preference. Move it out of
the list and say so.

**2 — Steelman.** Write the version of the proposal its author would sign. Fix its weak wording,
supply the argument it left implicit, and use its best evidence. Attacking a proposal you first
made weaker proves nothing.

**3 — Attack.** Read `references/lenses.md` and run each lens against every claim. Run the lenses
as parallel `general-purpose` subagents, one lens per agent, when the proposal is longer than a
page or the claims need files read. Run them inline otherwise.

**4 — Rank.** Sort every surviving finding into `fatal`, `structural`, `cost` or `cosmetic`.
Discard anything that only restates a claim in a hostile tone. Volume is not signal.

**5 — The core issue.** Name the single assumption that, if false, collapses the rest. Most
proposals have exactly one. Say so if this one has none.

**6 — Report.** Follow `references/output-format.md`. Every finding carries a remediation and a
test that would settle it.

## What counts as a finding

- A stated claim that is false, and the evidence that it is false.
- An unstated assumption the proposal needs and never argues.
- A step that does not follow from the step before it.
- A category error: the proposal treats one thing as another and inherits the wrong rules.
- A precedent that already went the other way, cited.

Not a finding: a restated risk with no mechanism, a preference dressed as a defect, a nitpick
about wording, or a demand for evidence the proposal never claimed to have.

## Evidence rule

The attack obeys the same bar it demands.

- A claim about a repository, a dataset or a document cites a path, a line or a command output.
- A claim about the world cites the document that was read.
- An objection with no pointer is marked `[unverified]` and is ranked no higher than `cost`.
- Never invent a failure. A hypothetical failure is written as a hypothesis with the test that
  would confirm it, never as an event.

## Hand off to council

Stop and use `council` when the answer to "so what should happen instead" is a choice between
options. Red-team can prove that a proposal is wrong. It is the wrong shape for deciding which
of three replacements is right, because it has no advocate for any of them.

## Error handling

- **The proposal survives.** Report that. A red-team pass that finds nothing fatal is a result,
  and inventing a finding to fill the report destroys the value of every other pass.
- **The proposal has no claims, only goals.** Say which claims are missing and stop. There is
  nothing to attack in a goal.
- **The caller asks to attack a person or a team.** Refuse and attack the proposal.
- **The caller wants a security scan of running software.** This skill attacks arguments. Say so.
