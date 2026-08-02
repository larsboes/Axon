// Every route here reads live state from capabilities on this machine, so there is
// nothing true at build time to prerender and nothing a server could render that the
// browser would not immediately replace.
export const ssr = false;
export const prerender = false;
