<!--
  One figure of a `Figures` row: its caption, its value (with a glyph when
  its state has one), and an optional quiet line under it. A figure that
  needs a look is tinted (`tone`).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Glyph from './Glyph.svelte';
  import type { Mark } from './marks.ts';

  let {
    label,
    mark = null,
    tone = null,
    title,
    href,
    children,
    sub,
  }: {
    label: string;
    /** The value's state, drawn before it. */
    mark?: Mark | null;
    tone?: 'warning' | 'error' | null;
    /** What the value counts, on hover. */
    title?: string;
    /** The page it leads to, when it leads somewhere. */
    href?: string;
    children: Snippet;
    sub?: Snippet;
  } = $props();
</script>

<svelte:element this={href ? 'a' : 'div'} class="figure {tone ?? ''}" {title} {href}>
  <span class="cap">{label}</span>
  <span class="figure-value mono"
    >{#if mark}<Glyph {mark} />{/if}{@render children()}</span
  >
  {#if sub}<span class="meta">{@render sub()}</span>{/if}
</svelte:element>

<style>
  .figure {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: 12px 16px;
    background: var(--figure-surface, var(--surface));
  }
  a.figure {
    color: inherit;
  }
  a.figure:hover {
    background: var(--surface-hover);
  }
  .figure-value {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
    font-size: 22px;
  }
  .warning {
    background: color-mix(in srgb, var(--warning) 8%, var(--figure-surface, var(--surface)));
  }
  .error {
    background: color-mix(in srgb, var(--error) 10%, var(--figure-surface, var(--surface)));
  }
  .warning .figure-value {
    color: var(--warning);
  }
  .error .figure-value {
    color: var(--error);
  }
</style>
