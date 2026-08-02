import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/**
 * adapter-static with an SPA fallback, not prerendering: every route here reads live
 * state from capabilities on this machine, so there is nothing true at build time to
 * render. The build is a static bundle any server can hand out, which is what makes
 * the eventual home-server deployment a file copy rather than a second architecture.
 *
 * @type {import("@sveltejs/kit").Config}
 */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({ pages: "dist", assets: "dist", fallback: "index.html", strict: false }),
  },
};

export default config;
