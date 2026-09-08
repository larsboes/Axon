/**
 * The shell's "start it once" guard, as a module the test can actually run.
 *
 * It lives outside `+layout.svelte` for one reason: a guard asserted against a copy of
 * itself is not asserted at all. `tools/dashboard-capability-start.test.ts` used to define
 * its own attempted-set beside the real one, so deleting the real one left every test
 * green. Both now call this.
 *
 * Imports nothing, so plain `bun test` can read it — the same rule `nav.ts` and
 * `home/decisions.ts` follow.
 */

export type ShellStarter = (
  names: readonly string[],
  worthStarting: (name: string) => boolean,
) => string[];

/**
 * Returns the capabilities a POST should be issued for, and records them as attempted.
 *
 * Both halves of "attempted" are load-bearing. The set is written BEFORE the caller posts,
 * so a capability that cannot start costs exactly one request rather than one every
 * fifteen seconds — `capabilities.svelte.ts` re-assigns its list on that interval, and an
 * effect that reads the store re-runs each time. And it is never cleared, because a start
 * that failed once will fail again for the same reason.
 *
 * `worthStarting` is asked only about a name that has not been attempted yet, and answers
 * false for a capability that is already up or that this build does not know about — an
 * unknown capability has no start route to post to.
 */
export function createShellStarter(): ShellStarter {
  const attempted = new Set<string>();
  return (names, worthStarting) => {
    const claimed: string[] = [];
    for (const name of names) {
      if (attempted.has(name)) continue;
      attempted.add(name);
      if (worthStarting(name)) claimed.push(name);
    }
    return claimed;
  };
}
