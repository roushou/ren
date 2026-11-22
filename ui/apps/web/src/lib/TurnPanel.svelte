<script lang="ts">
  /*
    One turn of a session, beside its calls: when it ran and how many calls
    it made, its context and cache against the report, what it was asked,
    the files and commands its calls named, and its findings; then the
    report, and the turn's own page.
  */
  import { ManagementApi, type TurnActivity, type TurnId, type TurnMetrics } from './api.ts';
  import { compact, span } from './display.ts';
  import { findingLine } from './activity.ts';
  import Glyph from './Glyph.svelte';
  import Icon from './Icon.svelte';
  import { Marks } from './marks.ts';

  let {
    turn,
    metrics,
    report,
  }: {
    turn: TurnId;
    /** The report's figures for it; null before the report reads. */
    metrics: TurnMetrics | null;
    /** The session's report page. */
    report: string;
  } = $props();

  const api = new ManagementApi();
  let read = $state.raw<{ turn: TurnId; activity: TurnActivity | null } | null>(null);
  $effect(() => {
    const target = turn;
    const abort = new AbortController();
    api.turn(target, abort.signal).then(
      (activity) => (read = { turn: target, activity }),
      () => (read = { turn: target, activity: null }),
    );
    return () => abort.abort();
  });
  const activity = $derived(read && read.turn === turn ? read.activity : null);
  const pending = $derived(!read || read.turn !== turn);

  const clock = (ms: number) =>
    new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const title = $derived(activity?.prompt?.text.trim().split('\n')[0] ?? 'Turn');
  const files = $derived(
    activity
      ? [
          ...activity.files_written.map((path) => ({ act: 'edit' as const, text: path })),
          ...activity.files_read
            .filter((path) => !activity.files_written.includes(path))
            .map((path) => ({ act: 'read' as const, text: path })),
          ...activity.commands.map((command) => ({ act: 'run' as const, text: command })),
        ]
      : [],
  );
  const findings = $derived((activity?.findings ?? []).map(findingLine));
  const cacheHit = $derived(metrics?.cache_hit_percent ?? null);
</script>

<div class="turn-panel">
  <header>
    <h2>{title}</h2>
    <p class="meta">
      {#if metrics}{clock(metrics.started_at)}{metrics.ended_at
          ? ` – ${clock(metrics.ended_at)}`
          : ' – now'} · {metrics.call_count} call{metrics.call_count === 1
          ? ''
          : 's'}{#if metrics.idle_before_ms}{' '}·
          <Glyph mark="idle" label="idle" />{span(metrics.idle_before_ms)} before{/if}{/if}
    </p>
  </header>

  {#if metrics}
    <div class="figures">
      <div>
        <span class="cap">Context</span><span class="figure"
          >{compact(metrics.context_start_tokens ?? 0)}→{compact(
            metrics.context_end_tokens ?? metrics.context_start_tokens ?? 0,
          )}</span
        >
      </div>
      <div>
        <span class="cap">From cache</span><span
          class="figure"
          class:low={cacheHit !== null && cacheHit < 50}
          >{cacheHit === null ? '—' : `${Math.round(cacheHit)}%`}</span
        >
      </div>
      <div>
        <span class="cap">Written</span><span class="figure"
          >{compact(metrics.cache_written_tokens)}</span
        >
      </div>
    </div>
  {/if}

  {#if pending}
    <p class="meta pad">Reading the turn…</p>
  {:else if !activity}
    <p class="meta pad">The turn did not read.</p>
  {:else}
    {#if activity.prompt}
      <section>
        <h3 class="cap">Prompt</h3>
        <p class="prompt">{activity.prompt.text}</p>
      </section>
    {/if}
    {#if files.length}
      <section>
        <h3 class="cap">Files and commands</h3>
        <ul class="files">
          {#each files as file, index (index)}
            <li>
              <span class="act" title={file.act}
                ><Icon
                  name={file.act === 'run' ? 'terminal' : file.act === 'edit' ? 'pencil' : 'file'}
                  size={12}
                /></span
              ><span class="mono">{file.text}</span>
            </li>
          {/each}
        </ul>
      </section>
    {/if}
    <section class="grow">
      <h3 class="cap">Findings in this turn · {findings.length}</h3>
      {#each findings as finding, index (index)}
        <div class="finding">
          <Glyph mark={Marks.ofTone(finding.tone)} label={true} />
          <span><span>{finding.title}</span><span class="meta">{finding.detail}</span></span>
        </div>
      {:else}<p class="meta">None.</p>{/each}
      <div class="actions">
        <a class="button small" href={report}>Open in report</a>
        <a class="button small" href="/monitor/turns/{encodeURIComponent(turn)}"
          >What this turn did</a
        >
      </div>
    </section>
  {/if}
</div>

<style>
  .turn-panel {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }
  header {
    padding: 12px 16px 10px;
    border-bottom: 1px solid var(--line);
  }
  h2 {
    margin: 0;
    font-size: 13.5px;
    font-weight: 600;
  }
  .meta {
    margin: 3px 0 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .pad {
    padding: 12px 16px;
  }
  .cap {
    margin: 0 0 8px;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .figures {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
    border-bottom: 1px solid var(--line);
    background: var(--line);
  }
  .figures > div {
    display: flex;
    flex-direction: column;
    padding: 10px 14px;
    background: var(--popover);
  }
  .figures .cap {
    margin: 0;
  }
  .figure {
    font-family: var(--mono);
    font-size: 15px;
  }
  .figure.low {
    color: var(--warning);
  }
  section {
    padding: 12px 16px;
    border-bottom: 1px solid var(--line-subtle);
  }
  .grow {
    flex: 1;
    border-bottom: 0;
  }
  .prompt {
    margin: 0;
    padding: 8px 10px;
    border: 1px solid var(--line);
    background: var(--surface-subtle);
    font-size: 12.5px;
    white-space: pre-wrap;
  }
  .files {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .files li {
    display: grid;
    grid-template-columns: 16px minmax(0, 1fr);
    gap: 8px;
    align-items: center;
  }
  .files .mono {
    overflow: hidden;
    font-family: var(--mono);
    font-size: 11px;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .act {
    color: var(--muted);
  }
  .finding {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr);
    gap: 8px;
    margin-bottom: 10px;
  }
  .finding > span {
    display: flex;
    flex-direction: column;
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 14px;
  }
</style>
