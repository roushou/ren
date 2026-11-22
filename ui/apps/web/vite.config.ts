import { defineConfig } from 'vite';
import { sveltekit } from '@sveltejs/kit/vite';
import adapter from '@sveltejs/adapter-node';
import { directives } from './src/csp.ts';

export default defineConfig({
  plugins: [
    sveltekit({
      adapter: adapter(),
      csp: {
        mode: 'nonce',
        directives,
      },
    }),
  ],
  build: {
    // Nothing becomes a data: URL: the policy (src/csp.ts) loads only from
    // this origin, so an inlined font would be refused.
    assetsInlineLimit: 0,
  },
});
