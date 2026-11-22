<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import SlideOver from '#lib/layer/SlideOver.svelte';
  import FloatingBar from '#lib/layer/FloatingBar.svelte';
  import Modal from '#lib/layer/Modal.svelte';
  import { toasts } from '#lib/layer/toasts.svelte.ts';
  import Toolbar from '#lib/Toolbar.svelte';
  import { onMount, untrack } from 'svelte';
  import SignInSheet from '#lib/SignInSheet.svelte';
  import { SignIns } from '#lib/signIn.ts';
  import { Runs, RunsClient, type Drivable, type RunClient } from '#lib/runs.ts';
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import { harnessLabel } from '#lib/activity.ts';
  import {
    KINDS,
    Platforms,
    RowActions,
    Supplies,
    ToolFilters,
    ToolGroups,
    type Only,
    ONLY_MARKS,
    ToolChanges,
    ToolPlan,
    ToolsApi,
    ToolsError,
    ToolHistory,
    type Generation,
    type GenerationView,
    type PackageId,
    type ToolPackage,
  } from '#lib/tools.ts';

  import type { PageProps } from './$types';
  import type { ToolsInitial } from './+page.server.ts';

  let { data }: PageProps = $props();
  const api = new ToolsApi(fetch);

  /** What the page shows: the daemon's listing, why there is none, or its
   * outline while the first read is on its way. */
  type Loaded = ToolsInitial | { kind: 'pending' };
  /** Whether a change is on its way; how it ended is a toast. */
  type Applying = { kind: 'idle' } | { kind: 'applying' };
  /** The change the operator is asked to confirm, if any. */
  type Confirming = { kind: 'apply' } | { kind: 'restore'; generation: Generation } | null;

  // The server's first read, done or on its way; later reads come from this page.
  let loaded = $state<Loaded>(
    untrack(() => (data.initial.kind === 'pending' ? { kind: 'pending' } : data.initial)),
  );
  let plan = $state(ToolPlan.EMPTY);
  let applying = $state<Applying>({ kind: 'idle' });
  let confirming = $state<Confirming>(null);
  /** The package whose details are open; none until one is chosen. */
  let selected = $state<PackageId | null>(null);

  // The first read, when it was handed over on its way.
  $effect(() => {
    const initial = data.initial;
    if (initial.kind !== 'pending') return;
    let current = true;
    void initial.read
      .catch((error: unknown): ToolsInitial => ({ kind: 'failed', message: message(error) }))
      .then((read) => {
        if (current) took(read);
      });
    return () => {
      current = false;
    };
  });
  let collecting = $state<string | null>(null);

  const listing = $derived(loaded.kind === 'ready' ? loaded.listing : null);
  let filter = $state(ToolFilters.ALL);
  const groups = $derived(listing ? ToolGroups.of(listing, filter) : []);
  const changes = $derived(listing ? plan.changes(listing) : []);
  const chosen = $derived(listing?.packages.find((p) => p.id === selected) ?? null);

  const message = (error: unknown) =>
    error instanceof ToolsError || error instanceof Error ? error.message : String(error);

  /** Show a read: its listing, keeping the plan within it, or why there is none. */
  function took(read: ToolsInitial) {
    loaded = read;
    if (read.kind === 'failed') return;
    plan = plan.within(read.listing);
  }

  async function load() {
    try {
      took({ kind: 'ready', listing: await api.listing() });
    } catch (error) {
      took({ kind: 'failed', message: message(error) });
    }
  }

  /** The outline's groups: the catalog's kinds, with rows varied in width. */
  const OUTLINE = KINDS.map((kind, index) => ({
    kind,
    title: ToolGroups.title(kind),
    rows: [3, 4, 3][index] ?? 3,
  }));
  const OUTLINE_WIDTHS = ['62%', '48%', '71%', '55%'] as const;

  async function apply() {
    confirming = null;
    if (plan.empty || applying.kind === 'applying') return;
    applying = { kind: 'applying' };
    try {
      const change = await api.change(plan);
      toasts.show(ToolChanges.said(change));
      if (change.outcome !== 'failed') plan = ToolPlan.EMPTY;
    } catch (error) {
      toasts.show({ mark: 'failed', text: 'The change was not applied', detail: message(error) });
    }
    applying = { kind: 'idle' };
    await load();
  }

  /** Make the next generation hold what `generation` held. */
  async function rollBack(generation: Generation) {
    confirming = null;
    if (applying.kind === 'applying') return;
    applying = { kind: 'applying' };
    try {
      toasts.show(ToolChanges.said(await api.rollBack(generation)));
      plan = ToolPlan.EMPTY;
    } catch (error) {
      toasts.show({
        mark: 'failed',
        text: `Generation ${generation} was not restored`,
        detail: message(error),
      });
    }
    applying = { kind: 'idle' };
    await load();
  }

  async function collect() {
    try {
      const collected = await api.collect();
      collecting = `${collected.generations.length + collected.installs.length} freed`;
    } catch (error) {
      collecting = message(error);
    }
    await load();
  }

  /** A row's button label, or `null` for a package Ren does not install. */
  function action(pkg: ToolPackage): string | null {
    const row = RowActions.of(pkg);
    if (row === null) return null;
    if (plan.staged(pkg)) return 'Undo';
    switch (row.kind) {
      case 'install':
        return `Install ${row.version}`;
      case 'update':
        return `Update ${row.version}`;
      case 'remove':
        return 'Remove';
    }
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
  const previous = $derived(
    listing?.generations
      .filter((generation) => generation.generation < listing.generation)
      .toSorted((a, b) => b.generation - a.generation)[0] ?? null,
  );
  const effects = $derived(listing ? plan.effects(listing) : []);
  const staged = $derived(new Map(effects.map((effect) => [effect.id, effect])));
  const short = (sha: string) => `${sha.slice(0, 12)}…`;

  // Each client's sign-in on this host, read from the run API when it is on.
  const runs = new RunsClient(fetch);
  // Read on the server with the listing when it could be; else here.
  let clients = $state.raw<RunClient[]>(untrack(() => data.clients ?? []));
  let signingIn = $state<Drivable | null>(null);
  let checking = $state<Drivable | null>(null);
  async function readClients() {
    try {
      clients = await runs.clients();
    } catch {
      // Runs are off or unreachable: no sign-in is shown.
    }
  }
  onMount(() => {
    if (untrack(() => data.clients) === null) void readClients();
  });
  /** The client a package is, with its sign-in; null for other packages. */
  const clientOf = (pkg: ToolPackage) => SignIns.clientOf(pkg, clients);
  const signedOut = $derived(
    listing?.packages.filter((pkg) => {
      const entry = clientOf(pkg);
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
    await readClients();
  }
  const stateLabel = (state: GenerationView['state']) =>
    state === 'current' ? 'current' : state === 'in_use' ? 'in use' : state === 'idle' ? '' : state;
</script>

{#snippet scope()}
  {#if listing}
    <span class="meta mono"
      >{listing.platform ? Platforms.label(listing.platform) : 'no installs on this platform'} · generation
      {listing.generation}{#if listing.bin}{' '}· {listing.bin}{/if}</span
    >
  {/if}
{/snippet}

{#snippet outline()}
  <!-- The listing on its way: the catalog's groups with rows where packages
       go, and the details pane's labels with bars where values go. -->
  <section class="tools-main" aria-label="Packages" aria-busy="true">
    <table class="tools-table">
      <colgroup>
        <col class="c-name" />
        <col class="c-version" />
        <col />
        <col class="c-action" />
      </colgroup>
      {#each OUTLINE as group (group.kind)}
        <tbody>
          <tr class="tools-group">
            <th colspan="4" scope="rowgroup">{group.title}</th>
          </tr>
          {#each { length: group.rows }, row (row)}
            <tr aria-hidden="true">
              <td class="mono name"
                ><Glyph mark="idle" /><Skeleton
                  width={OUTLINE_WIDTHS[row % OUTLINE_WIDTHS.length]}
                /></td
              >
              <td class="mono"><Skeleton width="4.5em" /></td>
              <td class="source"
                ><Skeleton width={OUTLINE_WIDTHS[(row + 2) % OUTLINE_WIDTHS.length]} /></td
              >
              <td class="end"></td>
            </tr>
          {/each}
        </tbody>
      {/each}
    </table>
  </section>
{/snippet}

{#snippet dot(pkg: ToolPackage)}
  {@const standing = Supplies.mark(pkg)}<Glyph mark={standing.mark} label={standing.word} />
{/snippet}

<Toolbar {scope} />
<div class="tools-view">
  {#if loaded.kind === 'failed'}
    <div class="tools-empty">
      <Icon name="alert" size={18} />
      <p>{loaded.message}</p>
      <button class="button" onclick={load}><Icon name="refresh" size={12} />Try again</button>
    </div>
  {:else if loaded.kind === 'pending'}
    {@render outline()}
  {:else if listing}
    <!-- The newest generations, as columns beside the packages. -->
    {@const history = new ToolHistory(listing)}
    <div class="tools-body">
      <div class="tools-left">
        {#if tally}
          <section class="figures" aria-label="Tools at a glance">
            <div>
              <span class="cap">Managed</span>
              <span class="figure"
                ><Glyph mark="passed" />{tally.managed}<span class="meta"
                  >in gen {listing.generation}</span
                ></span
              >
            </div>
            <div>
              <span class="cap">Updates</span>
              <span class="figure"
                >{#if updates.length}<Glyph mark="warning" />{/if}{updates.length}</span
              >
            </div>
            <div>
              <span class="cap">Missing</span>
              <span class="figure"
                >{#if missing.length}<Glyph mark="failed" />{/if}{missing.length}</span
              >
            </div>
            <div>
              <span class="cap">System</span>
              <span class="figure">{tally.system}<span class="meta">found on PATH</span></span>
            </div>
            <div class="generation-actions">
              {#if previous}<button
                  class="button small"
                  disabled={applying.kind === 'applying'}
                  onclick={() =>
                    (confirming = { kind: 'restore', generation: previous.generation })}
                  >Restore gen {previous.generation}</button
                >{/if}
              <button class="button small" onclick={collect}>Clean up</button>
              {#if collecting}<span class="meta">{collecting}</span>{/if}
            </div>
          </section>
          <div class="filters">
            <label class="search-input search">
              <Icon name="search" size={12} />
              <input
                type="search"
                placeholder="Filter"
                aria-label="Filter tools"
                value={filter.query}
                oninput={(event) => (filter = { ...filter, query: event.currentTarget.value })}
              />
            </label>
            {#snippet narrowing(only: Only | null, count: number, word: string, tone: string)}
              <button
                class="chip filter {tone}"
                class:active={filter.only === only}
                aria-pressed={filter.only === only}
                onclick={() => (filter = { ...filter, only })}
                >{#if only}<Glyph mark={ONLY_MARKS[only]} />{/if}{count}
                {word}</button
              >
            {/snippet}
            {@render narrowing(null, listing.packages.length, 'all', '')}
            {@render narrowing('managed', tally.managed, 'managed', 'success')}
            {@render narrowing('system', tally.system, 'system', '')}
            {#if tally.missing > 0}{@render narrowing(
                'missing',
                tally.missing,
                'missing',
                'error',
              )}{/if}
            {#if tally.updates > 0}{@render narrowing(
                'updates',
                tally.updates,
                tally.updates === 1 ? 'update' : 'updates',
                'live',
              )}{/if}
            {#if signedOut.length > 0}
              <button
                class="chip filter warning"
                onclick={() => (selected = signedOut[0]?.id ?? selected)}
                ><Icon name="lock" size={10} />{signedOut.length} signed out</button
              >
            {/if}
          </div>
        {/if}
        <section class="tools-main" aria-label="Packages">
          <table class="tools-table">
            <colgroup>
              <col class="c-name" />
              <col class="c-origin" />
              <col class="c-version" />
              <col />
              <col class="c-needed" />
              {#each history.generations as generation (generation.generation)}<col
                  class="c-gen"
                />{/each}
              <col class="c-action" />
            </colgroup>
            <thead>
              <tr>
                <th>Package</th>
                <th>Origin</th>
                <th>Version</th>
                <th>Path</th>
                <th>Needed by</th>
                {#each history.generations as generation (generation.generation)}
                  <th class="gen-head" class:current={generation.state === 'current'}
                    ><span class="mono">#{generation.generation}</span
                    >{#if generation.state === 'current'}<Glyph
                        mark="passed"
                        label={stateLabel(generation.state)}
                      />{:else if generation.state === 'in_use'}<Glyph
                        mark="responding"
                        label="In use by runs"
                      />{/if}{#if generation.state !== 'current' && listing.platform}<button
                        class="icon-button restore"
                        title="Restore generation {generation.generation}…"
                        aria-label="Restore generation {generation.generation}"
                        disabled={applying.kind === 'applying'}
                        onclick={() =>
                          (confirming = { kind: 'restore', generation: generation.generation })}
                        ><Icon name="refresh" size={11} /></button
                      >{/if}</th
                  >
                {/each}
                <th></th>
              </tr>
            </thead>
            {#each groups as group (group.kind)}
              <tbody>
                <tr class="tools-group">
                  <th colspan={6 + history.generations.length} scope="rowgroup"
                    >{group.title} <span class="count">{group.packages.length}</span></th
                  >
                </tr>
                {#each group.packages as pkg (pkg.id)}
                  {@const label = action(pkg)}
                  {@const supply = Supplies.of(pkg)}
                  {@const change = staged.get(pkg.id)}
                  <tr
                    class="tools-row"
                    class:selected={selected === pkg.id}
                    class:staged={change !== undefined}
                    onclick={() => (selected = pkg.id)}
                  >
                    <td class="mono name"
                      >{@render dot(pkg)}{pkg.id}{#if pkg.program !== pkg.id}<span
                          class="muted alias">{pkg.program}</span
                        >{/if}</td
                    >
                    <td class="mono muted"
                      >{pkg.origin === 'operator'
                        ? 'operator'
                        : pkg.origin === null
                          ? 'removed'
                          : pkg.origin}</td
                    >
                    <td class="mono"
                      >{#if change}<s class="muted">{change.from ?? '—'}</s><span class="arrow"
                          >→</span
                        ><span class="to {change.toKind}">{change.to ?? '—'}</span
                        >{:else if supply.kind === 'managed'}{supply.version}{:else if supply.kind === 'system'}{#if supply.system.version}{supply
                            .system.version}{:else}<span
                            class="unknown"
                            title={supply.system.unread ?? ''}>?</span
                          >{/if}{:else}<span class="muted">—</span>{/if}</td
                    >
                    <td class="mono muted path"
                      >{#if supply.kind === 'system'}{supply.system
                          .path}{:else if supply.kind === 'managed' && supply.shadows}<span
                          class="shadowed"
                          title="Runs find Ren’s first">{supply.shadows.path}</span
                        >{:else if Supplies.tone(pkg) === 'missing'}not found{:else}—{/if}</td
                    >
                    <td class="needed"
                      >{#each pkg.needed_by as harness (harness)}<span class="chip tag mono"
                          >{harnessLabel(harness).toLowerCase()}</span
                        >{/each}</td
                    >
                    {#each history.generations as generation, index (generation.generation)}
                      {@const held = history.version(pkg.id, index)}
                      <td
                        class="mono gen"
                        class:changed={history.changed(pkg.id, index)}
                        title={history.changed(pkg.id, index)
                          ? 'Changed in this generation'
                          : undefined}>{held ?? '—'}</td
                      >
                    {/each}
                    <td class="end">
                      {#if label && listing.platform}
                        <button
                          class="button small"
                          class:chosen={change !== undefined}
                          onclick={(event) => {
                            event.stopPropagation();
                            plan = plan.toggle(pkg);
                          }}>{label}</button
                        >
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            {:else}
              <tbody>
                <tr
                  ><td colspan={6 + history.generations.length} class="muted">No tool matches.</td
                  ></tr
                >
              </tbody>
            {/each}
          </table>
        </section>
      </div>
      {#if clients.length}
        <aside class="clients" aria-labelledby="clients-title">
          <h2 id="clients-title" class="cap">Clients · sign-in</h2>
          {#each clients as entry (entry.harness)}
            {@const harness = Runs.drivable(entry.harness)}
            {@const shown = SignIns.chip(entry.sign_in)}
            <div class="client">
              <Glyph
                mark={SignIns.signedOut(entry.sign_in)
                  ? 'warning'
                  : entry.sign_in.kind === 'signed_in'
                    ? 'passed'
                    : 'unknown'}
                label={shown?.text ?? 'Not read yet'}
              />
              <span>{harnessLabel(entry.harness)}</span>
              {#if harness}
                {#if SignIns.signedOut(entry.sign_in) && entry.sign_in_methods.length > 0}
                  <button class="button small" onclick={() => (signingIn = harness)}>Sign in</button
                  >
                {:else}
                  <button
                    class="button small"
                    disabled={checking === harness}
                    onclick={() => void check(harness)}
                    >{checking === harness ? 'Checking…' : 'Check again'}</button
                  >
                {/if}
              {/if}
            </div>
          {/each}
        </aside>
      {/if}
    </div>
    {#if chosen}
      {@const supply = Supplies.of(chosen)}
      {@const entry = clientOf(chosen)}
      {@const harness = entry ? Runs.drivable(entry.harness) : null}
      <SlideOver label="{chosen.id} details" pin="package" onclose={() => (selected = null)}
        ><div class="tools-inspector">
          <section class="head">
            <h2 class="mono">{@render dot(chosen)}{chosen.id}</h2>
            <span class="chip-row">
              {#if chosen.kind}<span class="chip">{chosen.kind}</span>{/if}
              {#if chosen.origin === 'operator'}<span class="chip">custom</span
                >{:else if chosen.origin === null}<Glyph
                  mark="warning"
                  label="Taken out of the catalog"
                />{/if}
            </span>
          </section>
          <section>
            <dl>
              <dt>Runs use</dt>
              <dd class="mono">
                {#if supply.kind === 'managed'}{supply.version}{:else if supply.kind === 'system'}{supply
                    .system.version ?? '?'}{:else}—{/if}
              </dd>
              {#if chosen.release}
                <dt>Pinned</dt>
                <dd class="mono">{chosen.release}</dd>
              {/if}
              {#if chosen.installed}
                <dt>SHA-256</dt>
                <dd class="mono" title={chosen.installed.sha256}>
                  {short(chosen.installed.sha256)}
                </dd>
                <dt>Install</dt>
                <dd class="mono">{chosen.installed.dir}</dd>
              {/if}
              {#if chosen.system}
                <dt>PATH</dt>
                <dd class="mono" class:shadowed={chosen.installed !== null}>
                  {chosen.system.path}
                  <span class="muted">{chosen.system.version ?? chosen.system.unread ?? ''}</span>
                </dd>
              {/if}
              {#if chosen.needed_by.length > 0}
                <dt>Needed by</dt>
                <dd>{chosen.needed_by.map(harnessLabel).join(', ')}</dd>
              {/if}
            </dl>
          </section>
          {#if entry && harness && (entry.sign_in.kind !== 'not_checked' || entry.sign_in_methods.length > 0)}
            {@const shown = SignIns.chip(entry.sign_in)}
            <section class="sign-in" class:out={SignIns.signedOut(entry.sign_in)}>
              <h3>
                Sign-in
                {#if shown}<span class="chip {shown.tone}">{shown.text}</span>{/if}
              </h3>
              {#if entry.sign_in.kind === 'unknown'}<p class="muted">{entry.sign_in.reason}</p>{/if}
              <div class="change-actions">
                <button
                  class="button small"
                  disabled={checking === harness}
                  onclick={() => void check(harness)}
                  >{checking === harness ? 'Checking…' : 'Check again'}</button
                >
                {#if entry.sign_in_methods.length > 0}
                  <button
                    class="button small"
                    class:primary={entry.sign_in.kind !== 'signed_in'}
                    onclick={() => (signingIn = harness)}
                    >{entry.sign_in.kind === 'signed_in' ? 'Sign in again' : 'Sign in'}</button
                  >
                {/if}
              </div>
            </section>
          {/if}
        </div></SlideOver
      >{/if}
    {#if applying.kind === 'applying'}<FloatingBar label="Changes"
        ><Glyph mark="running" /><span>Installing…</span></FloatingBar
      >{:else if !plan.empty}<FloatingBar label="Staged changes"
        ><Glyph mark="yours" /><b>{changes.length} staged</b><span class="mono muted staged"
          >{effects.map((e) => `${e.id} ${e.to ?? 'removed'}`).join(' · ')}</span
        ><button class="button small" onclick={() => (plan = ToolPlan.EMPTY)}>Clear</button><button
          class="button primary small"
          onclick={() => (confirming = { kind: 'apply' })}
          >Apply {changes.length} change{changes.length === 1 ? '' : 's'}…</button
        ></FloatingBar
      >{/if}
  {/if}
</div>

{#if confirming && listing}
  {@const asked = confirming}
  {#if asked.kind === 'apply'}
    <Modal
      eyebrow="Tools · {listing.platform ? Platforms.label(listing.platform) : 'this host'}"
      title="Apply {changes.length} change{changes.length === 1 ? '' : 's'}?"
      onclose={() => (confirming = null)}
    >
      <ul class="change-list">
        {#each effects as change (change.id)}
          <li class={change.change}>
            <span class="sign"
              >{change.change === 'add' ? '+' : change.change === 'remove' ? '−' : '↑'}</span
            >
            <span class="mono">{change.id}</span>
            <span class="mono muted">{change.from ?? '—'}</span>
            <span class="mono muted">→</span>
            <span class="mono to {change.toKind}">{change.to ?? '—'}</span>
          </li>
        {/each}
      </ul>
      <p class="muted">
        It becomes generation {listing.generation + 1}. Running sessions keep generation {listing.generation}
        until they start again, and generation {listing.generation} stays to restore.
      </p>
      {#snippet actions()}
        <button class="button" onclick={() => (confirming = null)}>Cancel</button>
        <button class="button primary" onclick={() => void apply()}>Apply</button>
      {/snippet}
    </Modal>
  {:else}
    {@const restored = listing.generations.find((g) => g.generation === asked.generation)}
    <Modal
      eyebrow="Tools · history"
      title="Restore generation {asked.generation}?"
      onclose={() => (confirming = null)}
    >
      <p>Runs started from now on get what it held:</p>
      <p class="mono">
        {restored?.installs.map((i) => `${i.package} ${i.version}`).join(', ') || 'nothing'}
      </p>
      <p class="muted">
        It becomes a new generation; generation {listing.generation} stays to restore.
      </p>
      {#snippet actions()}
        <button class="button" onclick={() => (confirming = null)}>Cancel</button>
        <button class="button primary" onclick={() => void rollBack(asked.generation)}
          >Restore</button
        >
      {/snippet}
    </Modal>
  {/if}
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
      void readClients();
    }}
  />
{/if}

<style>
  .tools-view {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    background: var(--surface);
  }
  .tools-empty {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 48px 16px;
  }
  .tools-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tally {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tally .chip :global(.glyph) {
    margin-right: 2px;
  }
  .tools-main {
    overflow: auto;
  }
  .tools-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 12px;
  }
  .c-name {
    width: 220px;
  }
  .c-version {
    width: 160px;
  }
  .c-action {
    width: 128px;
  }
  .c-origin {
    width: 90px;
  }
  .c-needed {
    width: 190px;
  }
  .alias {
    margin-left: 6px;
    font-size: 11px;
  }
  .needed {
    white-space: nowrap;
    overflow: hidden;
  }
  .needed .chip + .chip {
    margin-left: 3px;
  }
  .path {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .meta {
    color: var(--muted);
    font-size: 11px;
  }
  .cap {
    margin: 0;
    color: var(--muted);
    font-family: var(--mono);
    font-size: 10.5px;
    font-weight: 400;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .tools-body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
    min-height: 0;
    height: 100%;
  }
  .tools-left {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .figures {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr)) auto;
    gap: 1px;
    border-bottom: 1px solid var(--line);
    background: var(--line);
  }
  .figures > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 10px 16px;
    background: var(--surface);
  }
  .figure {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-family: var(--mono);
    font-size: 22px;
  }
  .figure .meta {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .generation-actions {
    flex-direction: row !important;
    align-items: center;
    gap: 6px !important;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
  }
  .filters .search {
    width: 220px;
  }
  .clients {
    min-height: 0;
    overflow: auto;
    padding: 12px 16px;
    border-left: 1px solid var(--line);
  }
  .clients .cap {
    margin-bottom: 8px;
  }
  .client {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    min-height: 32px;
  }
  @media (max-width: 1100px) {
    .tools-body {
      grid-template-columns: minmax(0, 1fr);
    }
    .clients {
      border-top: 1px solid var(--line);
      border-left: 0;
    }
    .figures {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  td,
  th {
    text-align: left;
    padding: 0 12px;
    height: 30px;
    vertical-align: middle;
    border-bottom: 1px solid var(--line);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tools-group th {
    height: 24px;
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    background: var(--surface-subtle);
  }
  .tools-group .count {
    font-weight: 400;
    margin-left: 4px;
  }
  .tools-row {
    cursor: pointer;
  }
  .tools-row:hover {
    background: var(--row-hover);
  }
  .tools-row.selected {
    background: var(--row-selected);
  }
  .tools-row.staged td:first-child {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  td.end {
    text-align: right;
    padding: 0 8px;
  }
  td.end .button {
    vertical-align: middle;
  }
  .name {
    font-weight: 600;
  }
  .arrow {
    margin: 0 4px;
    color: var(--muted);
  }
  .to.managed {
    color: var(--success);
  }
  .to.none {
    color: var(--error);
  }
  td > .chip {
    margin: 0;
    padding: 0 6px;
    font-size: 11px;
    line-height: 16px;
    vertical-align: 1px;
  }
  td > .tag {
    margin-left: 8px;
    font-weight: 400;
    font-family: inherit;
  }
  .source .path {
    margin-left: 8px;
  }
  .search {
    width: 160px;
    height: 24px;
    border: 1px solid var(--control-border);
    border-radius: 2px;
  }
  .chip.filter {
    cursor: pointer;
    font: inherit;
  }
  .chip.filter.active {
    box-shadow: inset 0 0 0 1px currentColor;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  .shadowed {
    text-decoration: line-through;
  }
  .unknown {
    color: var(--warning);
  }
  .name :global(.glyph),
  h2 :global(.glyph) {
    margin-right: 8px;
  }
  .staged {
    max-width: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .change-list {
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
  }
  .change-list li {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto auto auto;
    gap: 8px;
    padding: 3px 0;
  }
  .change-list .sign {
    font-weight: 700;
  }
  .change-list .add .sign {
    color: var(--success);
  }
  .change-list .replace .sign {
    color: var(--accent);
  }
  .change-list .remove .sign {
    color: var(--error);
  }
  .change-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .failed {
    color: var(--error);
  }
  .tools-inspector {
    min-height: 0;
    overflow: auto;
    font-size: 12px;
  }
  .sign-in p {
    margin: 4px 0 8px;
    line-height: 1.5;
  }
  .sign-in.out {
    background: var(--warning-soft);
  }
  .chip.sign-in {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 6px;
  }
  .tools-inspector section {
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
  }
  .tools-inspector h2 {
    margin: 0;
    font-size: 14px;
  }
  .head .chip-row {
    margin: 6px 0 0;
  }
  .gen-head {
    white-space: nowrap;
  }
  .gen-head > * {
    vertical-align: middle;
  }
  .gen-head :global(.glyph) {
    margin-left: 5px;
  }
  .restore {
    margin-left: 4px;
  }
  td.gen {
    color: var(--muted);
    font-size: 11px;
  }
  td.gen.changed {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--ink);
  }
  .tools-inspector h3,
  dl {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr);
    gap: 4px 8px;
    margin: 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
