# Output format

The report is the product. It is written so a reader six months later can re-open the question
without re-running the investigation.

## The report

```markdown
## Root cause: [the failure, as observed]

**Observed:** [what was seen, when it started, how it was noticed]
**Method:** [the method used, and why the evidence supported it]

### Timeline
| When | What | Source |
|---|---|---|
| [timestamp] | [the fact] | [log line, commit, path:line, command output] |

Last known good: [timestamp, and how that is known]
First known bad: [timestamp, and how that is known]

### Changes in the window
[each change, its reach, and whether it survived or was ruled out — with the reason]

### Candidate causes
| Candidate | Refuting evidence looked for | Result |
|---|---|---|
| [candidate] | [what would have killed it] | survived · refuted · untested |

### The cause
[One sentence: this condition existed, so that trigger produced this failure.]

**Falsifying probe:** [the command, query or observation whose result would prove that sentence
wrong, written so somebody else can run it]

**Contributing conditions:** [what made the trigger sufficient, and what delayed detection]

### Fixes
| Fix | Removes | Cost | Test that it worked |
|---|---|---|---|
| [fix] | this instance · this failure · this class | [cost] | [test] |

### Unverified and load-bearing
[each [unverified] claim the account rests on, and what would settle it]
```

## Rules

- **The cause is one sentence with a probe.** If it needs a paragraph, it is a story about the
  system, not a cause.
- **`untested` is an honest result** in the candidate table. Write it rather than promoting a
  candidate nobody tried to refute.
- **Every fix declares what it removes.** Instance, failure, or class — and a fix labelled `class`
  has to say which other failures it also prevents. That claim is checkable, which is the point.
- **No villain.** Name conditions, not people. An unattributable action is written as missing
  attribution, which is itself a finding and usually a fix.
- **Two causes stay two.** When the evidence supports both equally, report both with their probes
  and say which probe is cheaper. The reader picks.
- **The report ends open when the evidence ends.** A named cause nobody can falsify is worth less
  than an honest "the timeline stops here, and this is the instrumentation that would continue it".
