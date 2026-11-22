<!--
  System (`/system`): how the machine Ren runs on stands, one panel per
  area, each leading to its page. Recording: its verdict, its limits and
  what it counts since startup. Packages: how they stand and each client's
  sign-in. GitHub: whether it answers, and each watched repository's open
  pull requests.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Figures from '#lib/Figures.svelte';
  import Figure from '#lib/Figure.svelte';
  import Glyph from '#lib/Glyph.svelte';
  import Icon from '#lib/Icon.svelte';
  import Meter from '#lib/Meter.svelte';
  import PullBar from '#lib/PullBar.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { Now } from '#lib/now.svelte.ts';
  import { RecorderReport, type VerdictTone } from '#lib/recorder.ts';
  import { Supplies } from '#lib/packages.ts';
  import { SignIns } from '#lib/signIn.ts';
  import { GitHubWatches } from '#lib/github.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { Shares } from '#lib/display.ts';
  import type { Mark } from '#lib/marks.ts';
  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const clock = new Now(60_000);
  const gateway = createArrival(() => data.gateway);
  const listing = createArrival(() => data.listing);
  const clients = createArrival(() => data.clients);
  const github = createArrival(() => data.github);

  const report = $derived.by(() => {
    const read = gateway.snapshot?.value;
    return read
      ? new RecorderReport(
          read.health.recorder,
          read.storage.usage,
          read.storage.policy,
          null,
          clock.current,
        )
      : null;
  });
  const limits = $derived(report?.gauges.filter((gauge) => gauge.key !== 'history') ?? []);
  const tally = $derived(listing.snapshot?.value ? Supplies.tally(listing.snapshot.value) : null);
  const signIns = $derived(clients.snapshot?.value ?? null);
  const watches = $derived(
    github.snapshot?.value ? GitHubWatches.rows(github.snapshot.value.watches) : null,
  );
  const summary = $derived(watches ? GitHubWatches.summary(watches) : null);

  const MARKS: Record<VerdictTone, Mark> = {
    success: 'passed',
    warning: 'warning',
    error: 'failed',
  };
  /** A count that needs a look when it is not zero. */
  const flag = (count: number, tone: 'warning' | 'error') => (count ? tone : null);
</script>

{#snippet head(title: string, href: string, mark: Mark | null)}
  <a class="head" {href}>
    <h2 class="cap">{title}</h2>
    {#if mark}<Glyph {mark} />{/if}
    <Icon name="chevron" size={12} />
  </a>
{/snippet}

{#snippet waiting(error: { message: string } | null | undefined)}
  {#if error}
    <p class="failed" role="alert">{error.message}</p>
  {:else}
    <div class="rows" aria-busy="true">
      {#each { length: 3 }, row (row)}<div class="row"><Skeleton width="70%" /></div>{/each}
    </div>
  {/if}
{/snippet}

<Toolbar />
<div class="system">
  <section class="panel" aria-label="Recording">
    {@render head('Recording', '/system/recorder', report ? MARKS[report.verdict.tone] : null)}
    {#if report}
      <div class="gauges">
        {#each limits as gauge (gauge.key)}
          <div class="gauge">
            <span class="cap">{gauge.label}</span>
            <span class="mono gauge-value"
              >{gauge.used}<span class="meta">{' / '}{gauge.limit}</span></span
            >
            <span class="mono share">{gauge.share === null ? '' : Shares.percent(gauge.share)}</span
            >
            <Meter
              share={gauge.share ?? 0}
              size="bar"
              tone={RecorderReport.toneOf(RecorderReport.limitFullness(gauge))}
            />
          </div>
        {/each}
      </div>
      <div class="rows">
        {#each report.indicators as one (one.key)}
          <div
            class="row"
            class:warning={one.tone === 'warning'}
            class:error={one.tone === 'error'}
          >
            <Glyph mark={MARKS[one.tone]} />
            <span>{one.label}</span>
            <span class="mono value">{one.value}</span>
          </div>
        {/each}
      </div>
    {:else}
      {@render waiting(gateway.snapshot?.error)}
    {/if}
  </section>

  <section class="panel" aria-label="Packages">
    {@render head(
      'Packages',
      '/system/packages',
      tally ? (tally.missing ? 'failed' : tally.updates ? 'warning' : 'passed') : null,
    )}
    {#if tally}
      <Figures label="Packages" size="small">
        <Figure label="Managed">{tally.managed}</Figure>
        <Figure label="On PATH">{tally.system}</Figure>
        <Figure label="Updates" tone={flag(tally.updates, 'warning')}>{tally.updates}</Figure>
        <Figure label="Missing" tone={flag(tally.missing, 'error')}>{tally.missing}</Figure>
      </Figures>
    {:else}
      {@render waiting(listing.snapshot?.error)}
    {/if}
    {#if signIns}
      <h3 class="cap sub">Sign-in</h3>
      <div class="rows">
        {#each signIns as client (client.harness)}
          {@const chip = SignIns.chip(client.sign_in)}
          <div class="row" class:warning={SignIns.signedOut(client.sign_in)}>
            <Glyph
              mark={SignIns.signedOut(client.sign_in)
                ? 'warning'
                : client.sign_in.kind === 'signed_in'
                  ? 'passed'
                  : 'unknown'}
            />
            <span>{harnessLabel(client.harness)}</span>
            <span class="meta value">{chip?.text ?? ''}</span>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="panel" aria-label="GitHub">
    {@render head(
      'GitHub',
      '/system/github',
      summary ? (summary.failing ? 'failed' : summary.watched ? 'passed' : 'idle') : null,
    )}
    {#if summary && watches}
      <Figures label="GitHub" size="small">
        <Figure label="Answering" tone={flag(summary.failing, 'error')}
          >{summary.watched - summary.failing}/{summary.watched}</Figure
        >
        <Figure label="Open">{summary.open}</Figure>
        <Figure label="Checks failing" tone={flag(summary.checksFailing, 'error')}
          >{summary.checksFailing}</Figure
        >
      </Figures>
      <div class="rows">
        {#each watches.filter((row) => row.state !== 'off') as row (row.project)}
          <div class="row repo" class:error={row.mark === 'failed'}>
            <Glyph mark={row.mark} label={row.error ?? row.state} />
            <span class="mono ellipsis">{row.repository ?? row.project}</span>
            <PullBar {row} />
            <span class="mono value">{row.pulls.length}</span>
          </div>
        {/each}
      </div>
    {:else}
      {@render waiting(github.snapshot?.error)}
    {/if}
  </section>
</div>

<style>
  .system {
    display: grid;
    flex: 1;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 1px;
    min-height: 0;
    overflow: auto;
    background: var(--line);
  }
  .panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--surface);
    --figure-surface: var(--surface);
  }
  .head {
    display: flex;
    gap: 8px;
    align-items: center;
    height: 40px;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
  }
  .head h2 {
    color: var(--ink);
  }
  .head :global(svg:last-child) {
    margin-left: auto;
  }
  .head:hover {
    background: var(--surface-hover);
  }
  .gauges {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 14px 16px 16px;
    border-bottom: 1px solid var(--line);
  }
  .gauge {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    row-gap: 6px;
    align-items: baseline;
  }
  .gauge .cap {
    grid-column: 1 / -1;
  }
  .gauge-value {
    font-size: 20px;
  }
  .gauge-value .meta {
    font-size: 12px;
  }
  .share {
    color: var(--muted);
  }
  .gauge :global(.meter) {
    grid-column: 1 / -1;
  }
  .sub {
    padding: 14px 16px 4px;
    border-top: 1px solid var(--line);
  }
  .rows {
    padding: 4px 0;
  }
  .row {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    height: 32px;
    padding: 0 16px;
  }
  .row + .row {
    border-top: 1px solid var(--line-subtle);
  }
  .row.repo {
    grid-template-columns: 12px minmax(0, 1fr) minmax(48px, 30%) 2.5em;
  }
  .value {
    text-align: right;
  }
  .row.warning .value {
    color: var(--warning);
  }
  .row.error .value {
    color: var(--error);
  }
  .failed {
    margin: 0;
    padding: 12px 16px;
    color: var(--error);
  }
  @media (max-width: 1100px) {
    .system {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
