<!--
  A mark (`marks.ts`) drawn as its glyph: a shape per mark, so it reads
  without colour, in the mark's `--state` colour (style.css). With a label,
  the glyph is the thing said: it carries the label as its tooltip and its
  accessible name. Without one, words beside it say it, and it is hidden from
  assistive technology.
-->
<script lang="ts">
  import { Marks, type Mark } from './marks.ts';

  let {
    mark,
    label = null,
    size = 8,
  }: {
    mark: Mark;
    /** What the glyph says, when no words beside it do; `true` for the
     * mark's own word. */
    label?: string | true | null;
    size?: 8 | 10;
  } = $props();

  const said = $derived(label === true ? Marks.word(mark) : label);
</script>

{#if said}<span
    class="glyph state-{mark} {mark}"
    class:large={size === 10}
    role="img"
    aria-label={said}
    title={said}
  ></span>{:else}<span
    class="glyph state-{mark} {mark}"
    class:large={size === 10}
    aria-hidden="true"
  ></span>{/if}

<style>
  .glyph {
    display: inline-block;
    flex: none;
    box-sizing: border-box;
    width: 8px;
    height: 8px;
    vertical-align: 0;
  }
  .large {
    width: 10px;
    height: 10px;
  }
  .responding {
    border-radius: 50%;
    background: var(--state);
    animation: glyph-breathe 1.6s ease-in-out infinite;
  }
  .waiting,
  .slow {
    border: 1.5px solid var(--state);
    border-radius: 50%;
    animation: glyph-breathe 1.6s ease-in-out infinite;
  }
  .client {
    border: 1.5px solid var(--state);
    border-radius: 50%;
    background: linear-gradient(90deg, var(--state) 50%, transparent 50%);
  }
  .yours {
    background: var(--state);
    clip-path: polygon(0 0, 100% 50%, 0 100%);
  }
  .idle {
    height: 2px;
    vertical-align: 3px;
    background: var(--state);
  }
  .failed {
    background: var(--state);
    clip-path: polygon(
      20% 0,
      50% 30%,
      80% 0,
      100% 20%,
      70% 50%,
      100% 80%,
      80% 100%,
      50% 70%,
      20% 100%,
      0 80%,
      30% 50%,
      0 20%
    );
  }
  .warning {
    background: var(--state);
    clip-path: polygon(50% 0, 100% 100%, 0 100%);
  }
  .unknown {
    border: 1.5px dashed var(--state);
    border-radius: 50%;
  }
  .passed {
    background: var(--state);
    clip-path: polygon(0 55%, 14% 41%, 38% 64%, 86% 12%, 100% 26%, 38% 92%);
  }
  .running {
    border: 1.5px solid var(--line);
    border-top-color: var(--state);
    border-right-color: var(--state);
    border-radius: 50%;
    animation: glyph-spin 0.9s linear infinite;
  }
  .queued {
    border-radius: 50%;
    background: var(--state);
    transform: scale(0.5);
  }
  .draft {
    border: 1.5px dashed var(--state);
  }
  .info {
    background: var(--state);
    clip-path: polygon(50% 0, 100% 50%, 50% 100%, 0 50%);
  }
  @keyframes glyph-breathe {
    50% {
      opacity: 0.45;
    }
  }
  @keyframes glyph-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .glyph {
      animation: none;
    }
  }
</style>
