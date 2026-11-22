<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import Figures from '#lib/Figures.svelte';
  import Figure from '#lib/Figure.svelte';
  import FloatingBar from '#lib/layer/FloatingBar.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { invalidateAll } from '$app/navigation';
  import { createArrival } from '#lib/snapshot.svelte.ts';
  import SignInSheet from '#lib/SignInSheet.svelte';
  import { SignIns } from '#lib/signIn.ts';
  import { RunsClient, type Drivable } from '#lib/runs.ts';
  import Icon from '#lib/Icon.svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import {
    Platforms,
    RowActions,
    Supplies,
    PackageFilters,
    PackageGroups,
    PackageHistory,
    GenerationChanges,
    PackagePlan,
    PackagesApi,
    PackagesError,
    type Applying,
    type Confirming,
    type Generation,
    type PackageId,
  } from '#lib/packages.ts';
  import PackageTable from './PackageTable.svelte';
  import PackageFilterBar from './PackageFilterBar.svelte';
  import ClientsAside from './ClientsAside.svelte';
  import PackageDetails from './PackageDetails.svelte';
  import PackagesConfirm from './PackagesConfirm.svelte';

  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  const api = new PackagesApi(fetch);

  /** The daemon's listing: read, failed, or its outline while on its way. */
  const listingRead = createArrival(() => data.listing);
  const listing = $derived(listingRead.snapshot?.value ?? null);
  let plan = $state(PackagePlan.EMPTY);
  let applying = $state<Applying>({ kind: 'idle' });
  let confirming = $state<Confirming>(null);
  /** The package whose details are open; none until one is chosen. */
  let selected = $state<PackageId | null>(null);

  let collecting = $state<string | null>(null);

  let filter = $state(PackageFilters.ALL);
  const groups = $derived(listing ? PackageGroups.of(listing, filter) : []);
  /** The plan, kept within what the newest listing holds. */
  const kept = $derived(listing ? plan.within(listing) : plan);
  const changes = $derived(listing ? kept.changes(listing) : []);
  const chosen = $derived(listing?.packages.find((p) => p.id === selected) ?? null);

  async function apply() {
    confirming = null;
    if (kept.empty || applying.kind === 'applying') return;
    applying = { kind: 'applying' };
    try {
      const change = await api.change(kept);
      toasts.show(GenerationChanges.said(change));
      if (change.outcome !== 'failed') plan = PackagePlan.EMPTY;
    } catch (error) {
      toasts.show({
        mark: 'failed',
        text: 'The change was not applied',
        detail: PackagesError.say(error),
      });
    }
    applying = { kind: 'idle' };
    await invalidateAll();
  }

  /** Make the next generation hold what `generation` held. */
  async function rollBack(generation: Generation) {
    confirming = null;
    if (applying.kind === 'applying') return;
    applying = { kind: 'applying' };
    try {
      toasts.show(GenerationChanges.said(await api.rollBack(generation)));
      plan = PackagePlan.EMPTY;
    } catch (error) {
      toasts.show({
        mark: 'failed',
        text: `Generation ${generation} was not restored`,
        detail: PackagesError.say(error),
      });
    }
    applying = { kind: 'idle' };
    await invalidateAll();
  }

  async function collect() {
    try {
      const collected = await api.collect();
      collecting = `${collected.generations.length + collected.installs.length} freed`;
    } catch (error) {
      collecting = PackagesError.say(error);
    }
    await invalidateAll();
  }

  const tally = $derived(listing ? Supplies.tally(listing) : null);
  /** The packages each figure names. */
  const updates = $derived(
    listing?.packages.filter((pkg) => RowActions.of(pkg)?.kind === 'update').map((pkg) => pkg.id) ??
      [],
  );
  const missing = $derived(
    listing?.packages.filter((pkg) => Supplies.tone(pkg) === 'missing').map((pkg) => pkg.id) ?? [],
  );
  /** The generation before the current one, to restore. */
  const previous = $derived(listing ? PackageHistory.previous(listing) : null);
  const effects = $derived(listing ? kept.effects(listing) : []);
  const staged = $derived(new Map(effects.map((effect) => [effect.id, effect])));

  // Each client's sign-in on this host, read with the listing; none when runs
  // are off here.
  const runs = new RunsClient(fetch);
  const clientsRead = createArrival(() => data.clients);
  const clients = $derived(clientsRead.snapshot?.value ?? []);
  let signingIn = $state<Drivable | null>(null);
  let checking = $state<Drivable | null>(null);
  const signedOut = $derived(
    listing?.packages.filter((pkg) => {
      const entry = SignIns.clientOf(pkg, clients);
      return entry !== null && SignIns.signedOut(entry.sign_in);
    }) ?? [],
  );
  async function check(harness: Drivable) {
    checking = harness;
    try {
      await runs.checkSignIn(harness);
    } catch {
      // Shown as it was; the next read says.
    } finally {
      checking = null;
    }
    await invalidateAll();
  }
</script>

{#snippet scope()}
  {#if listing}
    <span class="meta mono"
      >{listing.platform ? Platforms.label(listing.platform) : 'no installs on this platform'} · generation
      {listing.generation}{#if listing.bin}{' '}· {listing.bin}{/if}</span
    >
  {/if}
{/snippet}

<Toolbar {scope} />
<div class="packages-view">
  {#if listingRead.current.state === 'pending'}
    <PackageTable rows={{ kind: 'outline' }} />
  {:else if listing}
    <div class="packages-body">
      <div class="packages-left">
        {#if tally}
          <Figures label="Packages at a glance">
            <Figure label="Managed" mark="passed">{tally.managed}</Figure>
            <Figure label="Generation">{listing.generation}</Figure>
            <Figure
              label="Updates"
              mark={updates.length ? 'warning' : null}
              tone={updates.length ? 'warning' : null}>{updates.length}</Figure
            >
            <Figure
              label="Missing"
              mark={missing.length ? 'failed' : null}
              tone={missing.length ? 'error' : null}>{missing.length}</Figure
            >
            <Figure label="On PATH">{tally.system}</Figure>
            {#snippet actions()}
              {#if previous}<button
                  class="button small"
                  disabled={applying.kind === 'applying'}
                  onclick={() =>
                    (confirming = { kind: 'restore', generation: previous.generation })}
                  >Restore gen {previous.generation}</button
                >{/if}
              <button class="button small" onclick={collect}>Clean up</button>
              {#if collecting}<span class="meta">{collecting}</span>{/if}
            {/snippet}
          </Figures>
          <PackageFilterBar
            bind:filter
            {tally}
            total={listing.packages.length}
            signedOut={signedOut.length}
            onsignedout={() => (selected = signedOut[0]?.id ?? selected)}
          />
        {/if}
        <PackageTable
          rows={{ kind: 'listing', listing, groups, plan: kept, staged, selected, applying }}
          onselect={(id) => (selected = id)}
          ontoggle={(pkg) => (plan = kept.toggle(pkg))}
          onrestore={(generation) => (confirming = { kind: 'restore', generation })}
        />
      </div>
      {#if clients.length}
        <ClientsAside
          {clients}
          {checking}
          oncheck={(harness) => void check(harness)}
          onsignin={(harness) => (signingIn = harness)}
        />
      {/if}
    </div>
    {#if chosen}
      <PackageDetails
        pkg={chosen}
        entry={SignIns.clientOf(chosen, clients)}
        {checking}
        oncheck={(harness) => void check(harness)}
        onsignin={(harness) => (signingIn = harness)}
        onclose={() => (selected = null)}
      />{/if}
    {#if applying.kind === 'applying'}<FloatingBar label="Changes"
        ><Glyph mark="running" /><span>Installing…</span></FloatingBar
      >{:else if !kept.empty}<FloatingBar label="Staged changes"
        ><Glyph mark="yours" /><b>{changes.length} staged</b><span class="mono muted staged"
          >{effects.map((e) => `${e.id} ${e.to ?? 'removed'}`).join(' · ')}</span
        ><button class="button small" onclick={() => (plan = PackagePlan.EMPTY)}>Clear</button
        ><button class="button primary small" onclick={() => (confirming = { kind: 'apply' })}
          >Apply {changes.length} change{changes.length === 1 ? '' : 's'}…</button
        ></FloatingBar
      >{/if}
  {:else}
    <div class="packages-empty">
      <Icon name="alert" size={18} />
      <p>{listingRead.snapshot?.error?.message ?? 'The packages did not read.'}</p>
      <button class="button" onclick={() => void invalidateAll()}
        ><Icon name="refresh" size={12} />Try again</button
      >
    </div>
  {/if}
</div>

{#if confirming && listing}
  <PackagesConfirm
    asked={confirming}
    {listing}
    {effects}
    count={changes.length}
    onapply={() => void apply()}
    onrestore={(generation) => void rollBack(generation)}
    onclose={() => (confirming = null)}
  />
{/if}

{#if signingIn}
  {@const harness = signingIn}
  <SignInSheet
    {harness}
    client={harnessLabel(harness)}
    methods={clients.find((entry) => entry.harness === harness)?.sign_in_methods ?? []}
    start={(method) => runs.startSignIn(harness, method)}
    answer={(answer) => runs.signInAnswer(harness, answer)}
    wait={() => runs.waitSignIn(harness)}
    cancel={() => runs.cancelSignIn(harness)}
    onclose={() => {
      signingIn = null;
      void invalidateAll();
    }}
  />
{/if}

<style>
  .packages-view {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    background: var(--surface);
  }
  .packages-empty {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 48px 16px;
  }
  .packages-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    min-height: 0;
    height: 100%;
  }
  .packages-left {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .packages-left > :global(.figures) {
    border-bottom: 1px solid var(--line);
  }
  .staged {
    max-width: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  @media (max-width: 1100px) {
    .packages-body {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
