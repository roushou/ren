import type { sveltekit } from '@sveltejs/kit/vite';

/** Kit's directive type, which it does not export by name. */
type CspDirectives = NonNullable<
  NonNullable<NonNullable<Parameters<typeof sveltekit>[0]>['csp']>['directives']
>;

/**
 * The UI's Content Security Policy, which SvelteKit sends with a per-response
 * nonce. Everything is same-origin; no inline script runs without the
 * per-response nonce, and no inline style runs.
 *
 * The one exception is SvelteKit's route announcer, the visually hidden
 * `<div id="svelte-announcer" style="…">` it renders for screen readers.
 * `style-src-attr` allows exactly that attribute value by its hash
 * (`'unsafe-hashes'` is what lets a hash apply to a style attribute), so no
 * other inline style can run. `tests/csp.test.ts` recomputes the hash from
 * the installed SvelteKit, so an upgrade that changes the style fails there.
 */
export const announcerStyleHash = 'sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo=' as const;

export const directives: CspDirectives = {
  'default-src': ['self'],
  'script-src': ['self'],
  'style-src': ['self'],
  'style-src-attr': ['unsafe-hashes', announcerStyleHash],
  'connect-src': ['self'],
  'object-src': ['none'],
  'base-uri': ['none'],
  'frame-ancestors': ['none'],
  'form-action': ['self'],
};
