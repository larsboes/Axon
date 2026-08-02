import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    // Match the proxy pattern: dev server proxies /api to itself
    proxy: {},
  },
});
