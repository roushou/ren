<!--
  A turn's heading in a session's listing: its prompt (which opens the turn's
  details), whether any of its calls failed, and how many calls it made.
-->
<script lang="ts">
  import Glyph from '../Glyph.svelte';
  import { Clock } from '../time.ts';
  import { CallTimeline } from '../timeline.ts';
  import type { TurnId } from '../api.ts';
  import type { TurnGroup } from '../traffic.ts';

  let {
    group,
    prompt,
    opened,
    columns,
    ontoggle,
  }: {
    group: TurnGroup;
    /** The turn's prompt, once read. */
    prompt: string | null | undefined;
    /** The turn open beside the calls. */
    opened: TurnId | null;
    /** How many columns the heading spans. */
    columns: number;
    /** Open a turn beside the calls, or close it. */
    ontoggle: (turn: TurnId | null) => void;
  } = $props();
  const turn = $derived(group.turn);
</script>

<tr class="turn-row" class:open={turn !== null && opened === turn}
  ><th colspan={columns}
    >{#if turn}<button
        class="turn-label"
        onclick={() => ontoggle(opened === turn ? null : turn)}
        aria-expanded={opened === turn}
        title="This turn's details">{prompt || group.label}</button
      >{:else}<span class="turn-label">{group.label}</span
      >{/if}{#if group.rows.some((call) => CallTimeline.mark(call).mark === 'failed')}<Glyph
        mark="failed"
        label={true}
      />{/if}<span class="muted mono"
      >{group.rows.length} call{group.rows.length === 1 ? '' : 's'} · {Clock.second(
        group.rows[0]?.started_at ?? 0,
      )}</span
    ></th
  ></tr
>

<style>
  .turn-row th {
    padding: 6px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-subtle);
    color: var(--ink);
    font-weight: 400;
    text-align: left;
  }
  .turn-row th > * {
    vertical-align: middle;
  }
  .turn-label {
    max-width: 70%;
    margin-right: 10px;
    overflow: hidden;
    font-size: 12.5px;
    font-weight: 500;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  button.turn-label {
    display: inline-block;
    text-align: left;
  }
  button.turn-label:hover {
    color: var(--accent);
  }
  .turn-row.open th {
    background: var(--row-selected);
  }
  .turn-row .muted {
    float: right;
    color: var(--muted);
    font-size: 11px;
  }
</style>
