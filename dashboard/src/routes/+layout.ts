// Every route here reads live state from capabilities on this machine, so there is
// nothing true at build time to prerender and nothing a server could render that the
// browser would not immediately replace.
import { base } from "$app/paths";
import { DEMO, installDemoFetch, type DemoIndex } from "$lib/demo";
import { setBase } from "$lib/nav";

// The single reader of `$app/paths` in this app. At module scope, so it runs before this
// module's `load` and long before any component renders a link. `base` is a build-time
// constant, so there is nothing to await and no ordering to get wrong.
setBase(base);

export const ssr = false;
export const prerender = false;

/**
 * The one place the demo's fetch shim can be installed.
 *
 * SvelteKit runs the root layout's load before any page load, which makes this the only hook
 * that is guaranteed to finish before a component asks a capability for anything. Installing
 * it from onMount would race: a page load already in flight would reach the real network,
 * which on a static host is a 404 rendered as "finance is not running".
 *
 * `DEMO` is a compile-time literal, so a normal build drops the import along with the branch.
 */
export async function load(): Promise<{ demo: DemoIndex | null }> {
  if (!DEMO) return { demo: null };
  return { demo: await installDemoFetch(base) };
}
