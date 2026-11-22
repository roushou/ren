<!--
  The session a call belongs to and who made it, in the call's row on the
  traffic page: each narrows the listing to its calls, and the session's card
  shows what it has done. A run of calls from one session names it once.
-->
<script lang="ts">
  import Glyph from '../Glyph.svelte';
  import Icon from '../Icon.svelte';
  import HoverCard from '../layer/HoverCard.svelte';
  import RecordingName from '../RecordingName.svelte';
  import { num, short, type CallView, type RecordingId, type RecordingView } from '../api.ts';
  import { compactOrDash, callerShown, recordingTone, SEPARATOR } from '../display.ts';
  import type { RecordingDetail } from '../recordings.ts';
  import type { Filters } from '../traffic.ts';

  let {
    call: c,
    previous,
    recordings,
    details,
    onfilter,
  }: {
    call: CallView;
    /** The row above, in the same group. */
    previous: CallView | undefined;
    /** The listed recordings by ID. */
    recordings: Map<RecordingId, RecordingView>;
    details: Map<RecordingId, RecordingDetail>;
    /** Narrow the listing and apply at once. */
    onfilter: (change: (filters: Filters) => void) => void;
  } = $props();
  /** Whether this call starts a run of its session's calls. */
  const first = $derived(!previous || previous.recording !== c.recording);
</script>

<small
  class="mono recording-line"
  title={c.attribution.hint?.session ?? 'No session identifier was reported'}
  >{#if c.recording}{@const recording = c.recording}{@const known =
      recordings.get(recording)}<HoverCard label="Recording {short(recording)}"
      >{#snippet trigger()}<button
          type="button"
          class="recording-filter"
          title="Show only this session's calls"
          aria-label={`Show only recording ${short(c.attribution.hint?.session ?? recording)}`}
          onclick={() => onfilter((f) => (f.recording = recording))}
          >{#if known && first}<RecordingName
              recording={known}
              detail={details.get(recording)}
              client={false}
            />{:else}<span class="recording-dot tone-{recordingTone(recording)}"
            ></span>{#if first}{c.attribution.hint
                ? short(c.attribution.hint.session)
                : 'Session'}{/if}{/if}</button
        >{/snippet}
      {#if known}
        <RecordingName recording={known} detail={details.get(recording)} />
        <span class="card-figures mono"
          ><span>{num(known.call_count)} calls</span><span>{num(known.turn_count)} turns</span
          >{#if known.error_count}<span class="card-errors"
              ><Glyph mark="failed" label="Failed calls" />{num(known.error_count)}</span
            >{/if}<span
            >{compactOrDash(known.tokens.input)} in · {compactOrDash(known.tokens.output)} out</span
          ></span
        >
        <a class="text-button" href="/monitor/sessions/{encodeURIComponent(recording)}"
          >Open the session <Icon name="arrow" size={11} /></a
        >
      {:else}<span class="muted">Not in this page's listing.</span>{/if}
    </HoverCard>{:else if first}No recording{/if}{#if c.caller.user || c.caller.login}{@const caller =
      callerShown(c.caller)}<span class="caller-line"
      >{SEPARATOR}{#if caller.user}{@const user = caller.user}<button
          type="button"
          class="recording-filter"
          title={`${caller.title}. Show only this user's calls`}
          aria-label={`Show only user ${user}'s calls`}
          onclick={() => onfilter((f) => (f.user = user))}>{caller.label}</button
        >{:else}<span title={caller.title}>{caller.label}</span>{/if}</span
    >{/if}</small
>

<style>
  /* One line a call: its ID, quiet, then the session it belongs to. */
  .recording-line {
    display: inline-flex;
    max-width: calc(100% - 76px);
    vertical-align: middle;
  }
  .recording-line :global(.recording-filter) {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-figures {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--muted);
    font-size: 11px;
  }
  .card-errors {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
