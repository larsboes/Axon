// tools/lib/ci-workflow.ts — the pure half of tools/ci-local: what CI declares, and which
// of it may be replayed on a machine that is not a disposable runner.
//
// Split out for the reason tools/lib/self-model.ts is: the launcher owns the I/O and the
// process, this owns the decisions, and `bun test` can import this without the CLI running
// itself on import.

/**
 * One `run:` step of a job, with the name the workflow gives it.
 *
 * `uses:` steps have no script and are absent from this type entirely — see parseJobs.
 */
export interface WorkflowStep {
  name: string;
  script: string;
}

export interface WorkflowJob {
  id: string;
  /** The job's `name:` in ci.yml, kept because that is the string a red check shows. */
  label: string;
  steps: WorkflowStep[];
  /** Null when the job may run here; otherwise the reason it must not. */
  refusal: string | null;
}

/**
 * Jobs whose steps may run against this machine exactly as CI wrote them, and why.
 *
 * The reason is stored rather than implied. It is what a reader has to weigh when a step
 * is added to one of these jobs, and it is the one thing a job name does not carry.
 */
export const RUNS_HERE: Record<string, string> = {
  "repo-gates": "every step is a file-based gate: no network, no build, no write into the checkout",
  "bun-tests": "bun test and tools/*.test.sh, which build their own scratch roots under /tmp",
};

/**
 * Jobs this refuses to replay here, each with the damage it would do.
 *
 * A GitHub runner is disposable. This machine holds the operator's overlay, the live
 * SQLite file eleven services share, and an Obsidian vault three capabilities project
 * into, so two of CI's four remaining jobs are destructive here and one is merely
 * expensive in the same place.
 */
export const REFUSED: Record<string, string> = {
  "rust-quality":
    "cargo against the shared target/ — a --release write there replaces a live service binary. Run: tools/cargo-hermetic clippy --workspace --all-targets --all-features --locked -- -D warnings",
  "cargo-tests":
    "cargo against the shared target/, and the suite resolves projection roots from the real overlay. Run: tools/cargo-hermetic test --workspace --locked",
  "ui-check":
    "its first step overwrites $HOME/.axon-overlay/config/machine.toml, this machine's own capability declaration, and its later steps bun install from the network. Run: cd <package> && bun run check",
};

/** What an unclassified job is told, so a new job in ci.yml is a decision and not a default. */
export const UNCLASSIFIED =
  "not classified — decide in tools/lib/ci-workflow.ts whether replaying it here is safe, and record why";

/**
 * Every job in a workflow, with its `run:` steps and its local verdict.
 *
 * `uses:` steps are dropped on purpose and that is not a gap: checkout, the two cargo
 * caches and setup-bun are all ways of getting a runner into the state this machine is
 * already in. A step with neither `run:` nor `uses:` is a malformed workflow and throws,
 * because skipping it would silently under-report the job — which is the failure this
 * whole tool exists to remove one level up.
 *
 * Classification is fail-closed: a job named in neither table is refused as unclassified.
 */
export function parseJobs(yamlText: string): WorkflowJob[] {
  const doc = Bun.YAML.parse(yamlText) as {
    jobs?: Record<string, { name?: string; steps?: Array<{ name?: string; run?: string; uses?: string }> }>;
  };
  const jobs = doc?.jobs;
  if (!jobs || typeof jobs !== "object") throw new Error("workflow declares no jobs");
  return Object.entries(jobs).map(([id, job]) => {
    const steps: WorkflowStep[] = [];
    for (const [i, step] of (job?.steps ?? []).entries()) {
      if (typeof step?.run === "string") {
        steps.push({ name: step.name ?? step.run.split("\n")[0], script: step.run });
      } else if (typeof step?.uses !== "string") {
        throw new Error(`${id}: step ${i + 1} has neither run: nor uses:`);
      }
    }
    return {
      id,
      label: job?.name ?? id,
      steps,
      refusal: id in RUNS_HERE ? null : (REFUSED[id] ?? UNCLASSIFIED),
    };
  });
}
