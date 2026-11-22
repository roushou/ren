<!--
  One package's details, over the page's right side: the version runs use,
  the release pinned, Ren's install and the PATH's copy, the clients that
  need it, and, for a client, its sign-in.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import SlideOver from '#lib/layer/SlideOver.svelte';
  import { SignIns } from '#lib/signIn.ts';
  import { Runs, type Drivable, type RunClient } from '#lib/runs.ts';
  import { harnessLabel } from '#lib/activity.ts';
  import { InstalledPackages, Supplies, type Package } from '#lib/packages.ts';
  import { dot } from './PackageTable.svelte';

  let {
    pkg,
    entry,
    checking,
    oncheck,
    onsignin,
    onclose,
  }: {
    pkg: Package;
    /** The client `pkg` is, with its sign-in; null for other packages. */
    entry: RunClient | null;
    /** The client whose sign-in is being checked, if any. */
    checking: Drivable | null;
    oncheck: (harness: Drivable) => void;
    onsignin: (harness: Drivable) => void;
    onclose: () => void;
  } = $props();

  const supply = $derived(Supplies.of(pkg));
  const harness = $derived(entry ? Runs.drivable(entry.harness) : null);
</script>

<SlideOver label="{pkg.id} details" pin="package" {onclose}
  ><div class="packages-inspector">
    <section class="head">
      <h2 class="mono">{@render dot(pkg)}{pkg.id}</h2>
      <span class="chip-row">
        {#if pkg.kind}<span class="chip">{pkg.kind}</span>{/if}
        {#if pkg.origin === 'operator'}<span class="chip">custom</span
          >{:else if pkg.origin === null}<Glyph
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
        {#if pkg.release}
          <dt>Pinned</dt>
          <dd class="mono">{pkg.release}</dd>
        {/if}
        {#if pkg.installed}
          <dt>SHA-256</dt>
          <dd class="mono" title={pkg.installed.sha256}>
            {InstalledPackages.shortSha(pkg.installed)}
          </dd>
          <dt>Install</dt>
          <dd class="mono">{pkg.installed.dir}</dd>
        {/if}
        {#if pkg.system}
          <dt>PATH</dt>
          <dd class="mono" class:shadowed={pkg.installed !== null}>
            {pkg.system.path}
            <span class="muted">{pkg.system.version ?? pkg.system.unread ?? ''}</span>
          </dd>
        {/if}
        {#if pkg.needed_by.length > 0}
          <dt>Needed by</dt>
          <dd>{pkg.needed_by.map(harnessLabel).join(', ')}</dd>
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
            onclick={() => oncheck(harness)}
            >{checking === harness ? 'Checking…' : 'Check again'}</button
          >
          {#if entry.sign_in_methods.length > 0}
            <button
              class="button small"
              class:primary={entry.sign_in.kind !== 'signed_in'}
              onclick={() => onsignin(harness)}
              >{entry.sign_in.kind === 'signed_in' ? 'Sign in again' : 'Sign in'}</button
            >
          {/if}
        </div>
      </section>
    {/if}
  </div></SlideOver
>

<style>
  .packages-inspector {
    min-height: 0;
    overflow: auto;
    font-size: 12px;
  }
  .packages-inspector section {
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
  }
  .packages-inspector h2 {
    margin: 0;
    font-size: 14px;
  }
  h2 :global(.glyph) {
    margin-right: 8px;
  }
  .head .chip-row {
    margin: 6px 0 0;
  }
  .shadowed {
    text-decoration: line-through;
  }
  .sign-in p {
    margin: 4px 0 8px;
    line-height: 1.5;
  }
  .sign-in.out {
    background: var(--warning-soft);
  }
  .change-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .packages-inspector h3,
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
