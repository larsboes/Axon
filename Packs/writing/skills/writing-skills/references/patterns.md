# Skill workflow shapes

## Contents
- Problem-first vs. tool-first framing
- Five shapes, pick the closest fit

## Problem-first vs. tool-first framing
Two ways a skill can be entered, and knowing which one fits changes what the body should say:
- **Problem-first.** The user describes an outcome ("set up a project workspace"); the skill orchestrates whatever calls/steps get there. Write for the outcome, hide the mechanism.
- **Tool-first.** The user already has the tool/capability in hand ("I have the Moonraker API connected"); the skill teaches the optimal sequence and gotchas for using it well. Write for the mechanism, the user already knows the goal.

Most skills lean one direction — decide which before drafting the body, since it changes whether Step 1 is "ask what the user wants" or "here's the command."

## Five shapes, pick the closest fit
Not mutually exclusive — a real skill is often one primary shape with a touch of another.

| Shape | Use when | Core technique |
|-------|----------|-----------------|
| **Sequential workflow** | Steps have a fixed, must-follow order | Numbered steps, explicit dependencies between them, validation at each stage, a rollback/undo note for failure mid-sequence |
| **Multi-tool/service coordination** | The task spans more than one system (e.g. a design tool → storage → task tracker → notification) | Clear phase separation, explicit data handoff between phases, validate before advancing to the next phase, one place that owns error handling |
| **Iterative refinement** | Output quality improves by looping, not by getting it right once (reports, generated docs) | A validation script or checklist run after each pass; explicit stop condition ("repeat until X passes") so the loop doesn't run forever |
| **Context-aware tool/path selection** | The same outcome needs a different tool or path depending on input (file size, type, environment) | A decision tree with named branches, a documented fallback when no branch matches, and the skill states *why* it picked a branch so the user isn't guessing |
| **Embedded domain intelligence** | The skill's value is the domain judgment itself, not tool access (e.g. a compliance check before a payment call) | The domain rule lives in the skill body (or `references/`, not off in someone's head), gated *before* the risky action, with an audit trail of what was decided and why |

For a runbook-style skill (drives one capability, like `home-3d-printing`), sequential workflow is almost always the right default — pick a different shape only when the task genuinely isn't a straight line.
