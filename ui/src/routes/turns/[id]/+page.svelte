<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, short, clock, title, type CallId } from '#lib/api.ts';
  import { compact, date, span, sessionTone } from '#lib/display.ts';
  import { effectLabel, findingLine, omitted, resultBadge } from '#lib/activity.ts';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.turn);
  const turn = $derived(snapshot.current.value);
  const findings = $derived(turn ? turn.findings.map(findingLine) : []);
  const clients: Record<string, string> = { claude_code: 'Claude Code', codex: 'Codex', pi: 'Pi' };
  const callHref = (call: CallId) =>
    turn ? `/sessions/${encodeURIComponent(turn.session)}?call=${encodeURIComponent(call)}` : '#';
  const tokens = (value: number | null) => (value === null ? '—' : compact(value));
</script>

{#snippet scope()}
  <div class="scope-group scope-identity">
    {#if turn}<span class="session-dot tone-{sessionTone(turn.session)}"></span>{/if}
    <strong class="mono" title="The turn ID your client reported"
      >{turn ? short(turn.native_id) : short(data.turnId)}</strong
    >
    <span class="muted"
      >Turn{turn?.client ? ` · ${clients[turn.client] ?? title(turn.client)}` : ''}{turn?.started_at
        ? ` · ${date(turn.started_at)} ${clock(turn.started_at)}`
        : ''}</span
    >
  </div>
  {#if turn}<div class="scope-group end">
      <a class="text-button" href={`/sessions/${encodeURIComponent(turn.session)}/report`}
        >Session report <Icon name="arrow" size={12} /></a
      ><a class="text-button" href={`/sessions/${encodeURIComponent(turn.session)}`}
        >Session calls <Icon name="arrow" size={12} /></a
      >
    </div>{/if}
{/snippet}

<div class="usage-page">
  <div class="console">
    {#if !turn}
      <div class="empty-state">
        <div class="empty-icon"><Icon name="alert" size={26} /></div>
        <h2>This turn could not be read</h2>
        <p>{snapshot.current.error?.message ?? 'Loading…'}</p>
      </div>
    {:else}
      <section class="panel usage-panel" aria-labelledby="turn-prompt-title">
        <header class="panel-heading">
          <h2 id="turn-prompt-title">Prompt</h2>
          <span class="muted"
            >{turn.started_at && turn.ended_at
              ? `Took ${span(turn.ended_at - turn.started_at)}`
              : 'Still running'}</span
          >
        </header>
        {#if turn.prompt}
          <p class="activity-text">{turn.prompt.text}</p>
          {#if turn.prompt.omitted_chars}<p class="activity-omitted">
              {omitted(turn.prompt.omitted_chars)}
            </p>{/if}
        {:else}
          <p class="quiet-empty">No prompt was read from this turn's requests.</p>
        {/if}
      </section>

      <section class="usage-tiles" aria-label="Totals">
        <div class="usage-tile">
          <span>Calls</span><strong class="mono"
            >{num(turn.calls.length + turn.omitted_calls)}</strong
          >
        </div>
        <div class="usage-tile">
          <span>Tool calls</span><strong class="mono">{num(turn.tool_calls)}</strong><small
            ><span class:error-count={turn.failed_tool_calls > 0}
              >{num(turn.failed_tool_calls)} failed</span
            >
            · {num(turn.unanswered_tool_calls)} without a result</small
          >
        </div>
        <div class="usage-tile">
          <span>Files written</span><strong class="mono">{num(turn.files_written.length)}</strong
          ><small>{num(turn.files_read.length)} read</small>
        </div>
        <div class="usage-tile">
          <span>Commands</span><strong class="mono">{num(turn.commands.length)}</strong>
        </div>
      </section>

      {#if findings.length}
        <section class="panel usage-panel" aria-labelledby="turn-findings-title">
          <header class="panel-heading"><h2 id="turn-findings-title">Findings</h2></header>
          <ul class="finding-list">
            {#each findings as line, index (index)}
              <li>
                <span class="chip {line.tone}">{line.title}</span>
                <span class="finding-detail">{line.detail}</span>
                {#if line.call}<a class="text-button" href={callHref(line.call)}
                    >Open call <Icon name="arrow" size={12} /></a
                  >{/if}
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if turn.files_written.length || turn.files_read.length || turn.commands.length}
        <div class="usage-columns">
          {#each [{ heading: 'Files written', items: turn.files_written }, { heading: 'Files read', items: turn.files_read }, { heading: 'Commands run', items: turn.commands }] as group (group.heading)}
            <section class="panel usage-panel">
              <header class="panel-heading"><h2>{group.heading}</h2></header>
              {#if group.items.length}
                <ul class="effect-list mono">
                  {#each group.items as item, index (index)}<li>{item}</li>{/each}
                </ul>
              {:else}<p class="quiet-empty">None.</p>{/if}
            </section>
          {/each}
        </div>
      {/if}

      <section class="panel usage-panel" aria-labelledby="turn-calls-title">
        <header class="panel-heading">
          <h2 id="turn-calls-title">Calls</h2>
          <span class="muted"
            >Tool calls are what the model requested; results are what the client reported</span
          >
        </header>
        {#each turn.calls as call (call.call)}
          <article class="activity-call">
            <header class="activity-call-head">
              <span class="mono">{clock(call.started_at)}</span>
              <span class="model-name">{call.model ?? 'No model reported'}</span>
              <span class="muted mono" title="Provider-reported tokens; not cost"
                >{tokens(call.metrics.context_tokens)} context · {tokens(
                  call.metrics.cache_written_tokens,
                )} written · {tokens(call.metrics.output_tokens)} out{call.metrics.duration_ms ===
                null
                  ? ''
                  : ` · ${span(call.metrics.duration_ms)}`}</span
              >
              {#each call.limitations as limitation (limitation)}<span class="chip warning"
                  >{title(limitation)}</span
                >{/each}
              <a class="text-button end" href={callHref(call.call)}
                >Inspect <Icon name="arrow" size={12} /></a
              >
            </header>
            {#if call.prompt && call.prompt.text !== turn.prompt?.text}
              <p class="activity-text activity-subprompt">{call.prompt.text}</p>
            {/if}
            {#if call.tool_calls.length}
              <table class="usage-list tool-table">
                <tbody>
                  {#each call.tool_calls as tool, index (tool.id ?? index)}
                    {@const badge = resultBadge(tool.result)}
                    <tr>
                      <td class="tool-name"
                        ><span><Icon name="tool" size={12} />{tool.name}</span></td
                      >
                      <td class="tool-target">
                        {#if tool.effects.length}
                          {#each tool.effects as effect, effectIndex (effectIndex)}<span
                              class="effect"
                              ><span class="muted">{effectLabel(effect.kind)}</span>
                              <span class="mono">{effect.target}</span></span
                            >{/each}
                        {:else}<details>
                            <summary class="muted">Input</summary>
                            <pre class="activity-pre">{tool.input.text}</pre>
                          </details>{/if}
                      </td>
                      <td><span class="chip {badge.tone}">{badge.label}</span></td>
                      <td class="tool-output">
                        {#if tool.result?.output.text}<details>
                            <summary class="muted">Output</summary>
                            <pre class="activity-pre">{tool.result.output.text}</pre>
                            {#if tool.result.output.omitted_chars}<p class="activity-omitted">
                                {omitted(tool.result.output.omitted_chars)}
                              </p>{/if}
                          </details>{/if}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            {/if}
            {#if call.reply}
              <p class="activity-text activity-reply">{call.reply.text}</p>
            {/if}
          </article>
        {/each}
        {#if turn.omitted_calls}<p class="quiet-empty">
            {num(turn.omitted_calls)} later calls beyond the read limit are not shown.
          </p>{/if}
      </section>
    {/if}
  </div>
</div>
