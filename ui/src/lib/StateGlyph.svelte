<script lang="ts">
  // A session state's glyph: a shape per state, so it reads without colour.
  // Its colour is the state's `--state` (style.css).
  import type { StateMark } from './board.ts';

  let { mark }: { mark: StateMark } = $props();
</script>

<span class="state-glyph state-{mark} {mark}" aria-hidden="true"></span>

<style>
  .state-glyph {
    display: inline-block;
    flex: none;
    box-sizing: border-box;
    width: 8px;
    height: 8px;
    margin-right: 6px;
    vertical-align: 0;
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
    background: var(--muted);
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
  @keyframes glyph-breathe {
    50% {
      opacity: 0.45;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .state-glyph {
      animation: none;
    }
  }
</style>
