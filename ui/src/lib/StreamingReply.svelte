<!--
  A call's reply while it streams: each update is revealed at a steady rate
  rather than pasted in at once. With reduced motion, or once the call has
  ended, updates show as they come. The caller draws the frame and caret.
-->
<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { prefersReducedMotion } from 'svelte/motion';
  import { Reveal } from './reveal.ts';
  import Markdown from './Markdown.svelte';

  let { text, writing, cut }: { text: string; writing: boolean; cut: boolean } = $props();

  // A card first drawn mid-stream shows what has arrived, then reveals the rest.
  const reveal = new Reveal(untrack(() => text));
  let shown = $state(reveal.text);
  let frame: number | undefined;
  let last = 0;

  function tick(now: number) {
    reveal.advance(last ? now - last : 0);
    last = now;
    shown = reveal.text;
    frame = reveal.done ? undefined : requestAnimationFrame(tick);
  }

  $effect(() => {
    reveal.update(text);
    if (prefersReducedMotion.current || !writing) {
      reveal.finish();
      shown = reveal.text;
      return;
    }
    if (frame === undefined) {
      last = 0;
      frame = requestAnimationFrame(tick);
    }
  });

  onDestroy(() => {
    if (frame !== undefined) cancelAnimationFrame(frame);
  });
</script>

{#if cut}<span class="cut">…</span>{/if}<Markdown text={shown} />

<style>
  .cut {
    color: var(--muted);
  }
</style>
