<!--
  A conversation on its way: turns ruled off as they will be, with bars
  where each prompt and reply go.
-->
<script lang="ts">
  import Skeleton from '#lib/Skeleton.svelte';

  /** The turns the outline draws: each prompt's width and its reply's
   * lines, varied so they read as text. */
  const OUTLINE_TURNS = [
    { prompt: '46%', reply: ['88%', '74%', '52%'] },
    { prompt: '31%', reply: ['81%', '63%'] },
    { prompt: '57%', reply: ['85%', '90%', '68%', '40%'] },
  ] as const;
</script>

{#each OUTLINE_TURNS as turn, index (index)}
  <section class="turn" aria-hidden="true">
    <div class="entry prompt outline"><Skeleton width={turn.prompt} /></div>
    <div class="entry reply outline">
      {#each turn.reply as width, line (line)}<Skeleton {width} />{/each}
    </div>
  </section>
{/each}

<style>
  .turn {
    position: relative;
    border-bottom: 1px solid var(--line);
  }
  .entry {
    padding: 0 16px;
    font-size: 13px;
  }
  /* An entry's outline: its bars stacked as its lines will be. */
  .entry.outline {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 7px;
    padding-block: 12px;
  }
  /* A prompt is drawn on the inset surface the bars use: its bar is ruled. */
  .entry.prompt.outline :global(.skeleton) {
    background: var(--line);
  }
  .entry.prompt {
    background: var(--surface-inset);
    box-shadow: inset 2px 0 var(--role-user);
    border-bottom: 1px solid var(--line);
  }
  .entry.reply {
    padding-top: 10px;
    padding-bottom: 10px;
    box-shadow: inset 2px 0 var(--role-assistant);
  }
</style>
