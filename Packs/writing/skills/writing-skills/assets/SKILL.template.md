---
name: [skill-name]
description: [Third-person capability statement. What it does + "Use when <triggers/contexts>" + "Do not use for <negative triggers>". <= 1024 chars, no XML tags, no first/second person.]
allowed-tools: [optional, Claude Code only — e.g. Read, Write, Bash(git status:*)]
---

# [Skill Title]

[One or two sentences: the specialist capability this adds. Assume Claude is already smart — only add what it can't infer.]

## [Workflow name]

Copy this checklist and track progress:

```
- [ ] Step 1: [action]
- [ ] Step 2: [action]
- [ ] Step 3: [validate]
```

**Step 1 — [Action phase]**
1. [Third-person imperative instruction.]
2. [If a large schema/rule-set is needed: "Read `references/<file>.md` to <purpose>." — one level deep, at point of need.]

**Step 2 — [Action phase]**
1. [Decision point: "If <condition>, run `scripts/<script>.py`. Otherwise skip to Step 3."]
2. Run `scripts/<script>.py <args>` to [deterministic action]. [State execute-vs-read intent explicitly.]

## Error handling
* If `scripts/<script>.py` fails on [edge case], [recovery step].
* If [condition], read `references/<troubleshooting>.md`.
