<script lang="ts">
  import { createSnapshot } from '#lib/snapshot.svelte.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { num, short, clock, title } from '#lib/api.ts';
  import { compactOrDash, date, span, sessionTone } from '#lib/display.ts';
  import { callHref, effectLabel, findingLine, harnessLabel, omitted } from '#lib/activity.ts';
  import { Board } from '#lib/board.ts';
  import FindingList from '#lib/FindingList.svelte';
  import Icon from '#lib/Icon.svelte';
  import type { PageProps } from './$types';
  let { data }: PageProps = $props();
  getWorkbench().showScope(scope);
  const snapshot = createSnapshot(() => data.turn);
  const turn = $derived(snapshot.current.value);
  const findings = $derived(turn ? turn.findings.map(findingLine) : []);
  /** The folder the turn's commands work in: paths under it are shown relative to it. */
  const workdir = $derived(turn ? Board.workdir(turn) : null);
  const shorten = (target: string) => Board.shorten(target, workdir);
  /** A command's first line, and whether there is more to open. */
  function commandLine(command: string): { first: string; more: boolean } {
    const text = shorten(command);
    const first = text.split('\n')[0] ?? text;
    return { first, more: first !== text.trimEnd() };
  }
</script>

{#snippet scope()}
  <div class="scope-group scope-identity">
    {#if turn}<span class="session-dot tone-{sessionTone(turn.session)}"></span>{/if}
    <strong class="mono" title="The turn ID your client reported"
      >{turn ? short(turn.native_id) : short(data.turnId)}</strong
    >
    <span class="muted"
      >Turn{turn?.client ? ` · ${harnessLabel(turn.client)}` : ''}{turn?.started_at
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
          {#if workdir}<span class="muted mono turn-folder" title={workdir}
              >in {Board.folder(workdir)}</span
            >{/if}
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
          <FindingList {findings} session={turn.session} page="turn" />
        </section>
      {/if}

      {#if turn.files_written.length || turn.files_read.length || turn.commands.length}
        <div class="usage-columns">
          {#each [{ heading: 'Files written', items: turn.files_written }, { heading: 'Files read', items: turn.files_read }] as group (group.heading)}
            <section class="panel usage-panel">
              <header class="panel-heading"><h2>{group.heading}</h2></header>
              {#if group.items.length}
                <ul class="effect-list mono">
                  {#each group.items as item, index (index)}<li title={item}>
                      {shorten(item)}
                    </li>{/each}
                </ul>
              {:else}<p class="quiet-empty">None.</p>{/if}
            </section>
          {/each}
          <section class="panel usage-panel">
            <header class="panel-heading"><h2>Commands run</h2></header>
            {#if turn.commands.length}
              <ul class="effect-list mono command-list">
                {#each turn.commands as command, index (index)}
                  {@const line = commandLine(command)}
                  <li>
                    {#if line.more}<details>
                        <summary title="Show the whole command">{line.first}</summary>
                        <pre class="activity-pre">{shorten(command)}</pre>
                      </details>{:else}<span class="command-text" title={command}>{line.first}</span
                      >{/if}
                  </li>
                {/each}
              </ul>
            {:else}<p class="quiet-empty">None.</p>{/if}
          </section>
        </div>
      {/if}

      <section class="panel usage-panel" aria-labelledby="turn-calls-title">
        <header class="panel-heading">
          <h2 id="turn-calls-title">Calls</h2>
          <span class="muted"
            >Tool calls are what the model requested; results are what the client reported</span
          >
        </header>
        {#each turn.calls as call, callIndex (call.call)}
          <article class="activity-call">
            <header class="activity-call-head">
              <span class="mono">{clock(call.started_at)}</span>
              <span class="model-name">{call.model ?? 'No model reported'}</span>
              <span class="muted mono" title="Provider-reported tokens; not cost"
                >{compactOrDash(call.metrics.context_tokens)} context · {compactOrDash(
                  call.metrics.cache_written_tokens,
                )} written · {compactOrDash(call.metrics.output_tokens)} out{call.metrics
                  .duration_ms === null
                  ? ''
                  : ` · ${span(call.metrics.duration_ms)}`}</span
              >
              {#each call.limitations as limitation (limitation)}<span
                  class="chip neutral"
                  title="What Ren could not read of this call's stored text; forwarding was not affected"
                  >{title(limitation)}</span
                >{/each}
              <a class="text-button end" href={callHref(turn.session, call.call)}
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
                    {@const badge = Board.result(turn, callIndex, tool)}
                    <tr>
                      <td class="tool-name"
                        ><span><Icon name="tool" size={12} />{tool.name}</span></td
                      >
                      <td class="tool-target">
                        {#if tool.effects.length}
                          {#each tool.effects as effect, effectIndex (effectIndex)}<span
                              class="effect"
                              ><span class="muted">{effectLabel(effect.kind)}</span>
                              <span class="mono" title={effect.target}
                                >{shorten(effect.target)}</span
                              ></span
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

<style>
  .turn-folder {
    margin-right: auto;
    font-size: 11px;
  }
  /* One row per command: its first line, opening to the whole of it. */
  .command-list :global(li) {
    overflow: hidden;
  }
  .command-list summary,
  .command-text {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .command-text {
    cursor: text;
  }
  .command-list summary::marker {
    color: var(--muted);
  }
  .command-list details[open] summary {
    white-space: normal;
  }
</style>
