# Lenses

Six angles of attack. Each one asks a different question, so each one finds a different class of
defect. Run all six. Six lenses that disagree beat thirty that repeat each other: the upstream
this skill was adapted from ran 32 agents in four types, and the extra copies produced volume to
rank rather than defects to fix.

Run one lens per `general-purpose` subagent, all in a single message, when the proposal is longer
than a page or the claims need files read. Run them inline otherwise.

Give every lens the same input: the atomic claim list, the steelman, the evidence gathered, and
its own brief below.

## The six

**Engineer — does the mechanism work?**
Attacks the step from input to result. Asks what the numbers are, what the edge case does, and
what happens at ten times the stated load. Demands the measurement, the benchmark or the code
path. Finds: claims that are false, and steps that do not follow.

**Architect — does it fit what already exists?**
Attacks the shape rather than the mechanism. Asks what else has to change, what this duplicates,
and what it makes impossible later. Demands the current structure and the call sites. Finds:
structural defects and category errors.

**Operator — what happens when it breaks at 03:00?**
Attacks the run-time life of the proposal. Asks how it fails, who is paged, what the signal is,
and how it is reverted. Demands the failure mode and the rollback path. Finds: cost findings that
the design discussion never surfaces, and occasionally a fatal one.

**Adversary — who profits from this failing?**
Attacks the proposal as a target. Asks who is motivated to abuse it, what the cheapest abuse is,
and what it grants that it did not intend to grant. Demands the trust boundary and the scope of
anything it hands out. Finds: abuse paths and misplaced trust.

**Newcomer — what does this assume the reader already knows?**
Attacks the unstated. Asks what a competent person reading this for the first time cannot follow,
which term is used in two senses, and which step is skipped because it is obvious to the author.
Demands nothing, which is the point. Finds: hidden assumptions, and this lens finds more fatal
issues than its naive framing suggests.

**Economist — what does it cost and who pays?**
Attacks the price. Asks what the total cost is over the horizon, what the cheaper alternative
would have achieved, and who bears the cost that the proposer does not. Demands the cost figures
and the do-nothing baseline. Finds: cost findings, and the occasional fatal one when the do-
nothing baseline wins.

## Adding a lens

Add a seventh only when the proposal has a domain none of the six can read — a regulator, a
clinician, a specific market. Write it in the same shape: what it attacks, what it asks, what it
demands, what it finds. Do not add a lens whose only difference is a job title.
