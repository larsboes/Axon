import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
  kit: {
    adapter: adapter({
      pages: 'dist',
      assets: 'dist',
      precompress: true,
    }),
    prerender: {
      handleHttpError: 'warn',
    },
  },
};

export default config;
