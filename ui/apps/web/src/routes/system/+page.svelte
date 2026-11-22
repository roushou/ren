<!--
  System (`/system`): how the machine Ren runs on stands. Recording, the
  packages Ren installs and the clients' sign-ins, and the GitHub watches,
  each a row of figures leading to its page.
-->
<script lang="ts">
  import Toolbar from '#lib/Toolbar.svelte';
  import Figures from '#lib/Figures.svelte';
  import Figure from '#lib/Figure.svelte';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import { Now } from '#lib/now.svelte.ts';
  import { RecorderReport } from '#lib/recorder.ts';
  import { Supplies } from '#lib/packages.ts';
  import { SignIns } from '#lib/signIn.ts';
  import { GitHubWatches } from '#lib/github.ts';
  import { Shares } from '#lib/display.ts';
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
  const indicators = $derived(
    report?.indicators.filter((one) => one.key === 'lost' || one.key === 'errors') ?? [],
  );
  const limits = $derived(report?.gauges.filter((gauge) => gauge.key !== 'history') ?? []);
  const tally = $derived(listing.snapshot?.value ? Supplies.tally(listing.snapshot.value) : null);
  const signedOut = $derived(
    (clients.snapshot?.value ?? []).filter((client) => SignIns.signedOut(client.sign_in)).length,
  );
  const watches = $derived.by(() => {
    const read = github.snapshot?.value;
    return read ? GitHubWatches.summary(GitHubWatches.rows(read.watches)) : null;
  });
  const tone = (bad: boolean, how: 'warning' | 'error') => (bad ? how : null);
</script>

<Toolbar />
<div class="system">
  <section aria-labelledby="recording-title">
    <h2 id="recording-title" class="cap">Recording</h2>
    <Figures label="Recording">
      {#each indicators as one (one.key)}
        <Figure
          label={one.label}
          mark={one.tone === 'success' ? null : one.tone === 'warning' ? 'warning' : 'failed'}
          tone={one.tone === 'success' ? null : one.tone}
          href="/system/recorder">{one.value}</Figure
        >
      {:else}
        <Figure label="Recording" href="/system/recorder">—</Figure>
      {/each}
      {#each limits as gauge (gauge.key)}
        {@const fullness = RecorderReport.limitFullness(gauge)}
        <Figure
          label={gauge.label}
          tone={fullness === 'fine' ? null : fullness}
          title="{gauge.used} of {gauge.limit}"
          href="/system/recorder"
          >{gauge.share === null ? gauge.used : Shares.percent(gauge.share)}</Figure
        >
      {/each}
    </Figures>
  </section>

  <section aria-labelledby="packages-title">
    <h2 id="packages-title" class="cap">Packages</h2>
    {#if tally}
      <Figures label="Packages">
        <Figure label="Managed" href="/system/packages">{tally.managed}</Figure>
        <Figure
          label="Updates"
          mark={tally.updates ? 'warning' : null}
          tone={tone(tally.updates > 0, 'warning')}
          href="/system/packages">{tally.updates}</Figure
        >
        <Figure
          label="Missing"
          mark={tally.missing ? 'failed' : null}
          tone={tone(tally.missing > 0, 'error')}
          href="/system/packages">{tally.missing}</Figure
        >
        <Figure
          label="Signed out"
          mark={signedOut ? 'warning' : null}
          tone={tone(signedOut > 0, 'warning')}
          href="/system/packages">{signedOut}</Figure
        >
      </Figures>
    {:else}
      <p class="meta pad">{listing.snapshot?.error?.message ?? '—'}</p>
    {/if}
  </section>

  <section aria-labelledby="github-title">
    <h2 id="github-title" class="cap">GitHub</h2>
    {#if watches}
      <Figures label="GitHub">
        <Figure
          label="Answering"
          mark={watches.failing ? 'failed' : watches.watched ? 'passed' : 'idle'}
          tone={tone(watches.failing > 0, 'error')}
          href="/system/github">{watches.watched - watches.failing}/{watches.watched}</Figure
        >
        <Figure label="Open pull requests" href="/system/github">{watches.open}</Figure>
        <Figure
          label="Checks failing"
          tone={tone(watches.checksFailing > 0, 'error')}
          href="/system/github">{watches.checksFailing}</Figure
        >
      </Figures>
    {:else}
      <p class="meta pad">{github.snapshot?.error?.message ?? '—'}</p>
    {/if}
  </section>
</div>

<style>
  .system {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    overflow: auto;
    background: var(--line);
  }
  section {
    background: var(--surface);
  }
  section:last-child {
    flex: 1;
  }
  h2 {
    padding: 12px 16px 8px;
  }
  .pad {
    margin: 0;
    padding: 0 16px 12px;
  }
</style>
