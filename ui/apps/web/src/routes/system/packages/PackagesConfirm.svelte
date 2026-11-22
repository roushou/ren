<!--
  The change the operator is asked to confirm: applying the staged plan,
  which lists each change runs will see, or restoring a kept generation,
  which says what it held.
-->
<script lang="ts">
  import Modal from '#lib/layer/Modal.svelte';
  import {
    ChangeKinds,
    Platforms,
    type Confirming,
    type Effect,
    type Generation,
    type PackageListing,
  } from '#lib/packages.ts';

  let {
    asked,
    listing,
    effects,
    count,
    onapply,
    onrestore,
    onclose,
  }: {
    asked: NonNullable<Confirming>;
    listing: PackageListing;
    /** The staged plan's changes, as runs see them. */
    effects: Effect[];
    /** How many changes the staged plan makes. */
    count: number;
    onapply: () => void;
    onrestore: (generation: Generation) => void;
    onclose: () => void;
  } = $props();
</script>

{#if asked.kind === 'apply'}
  <Modal
    eyebrow="Packages · {listing.platform ? Platforms.label(listing.platform) : 'this host'}"
    title="Apply {count} change{count === 1 ? '' : 's'}?"
    {onclose}
  >
    <ul class="change-list">
      {#each effects as change (change.id)}
        <li class={change.change}>
          <span class="sign">{ChangeKinds.sign(change.change)}</span>
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
      <button class="button" onclick={onclose}>Cancel</button>
      <button class="button primary" onclick={onapply}>Apply</button>
    {/snippet}
  </Modal>
{:else}
  {@const restored = listing.generations.find((g) => g.generation === asked.generation)}
  <Modal eyebrow="Packages · history" title="Restore generation {asked.generation}?" {onclose}>
    <p>Runs started from now on get what it held:</p>
    <p class="mono">
      {restored?.installs.map((i) => `${i.package} ${i.version}`).join(', ') || 'nothing'}
    </p>
    <p class="muted">
      It becomes a new generation; generation {listing.generation} stays to restore.
    </p>
    {#snippet actions()}
      <button class="button" onclick={onclose}>Cancel</button>
      <button class="button primary" onclick={() => onrestore(asked.generation)}>Restore</button>
    {/snippet}
  </Modal>
{/if}

<style>
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
  .to.managed {
    color: var(--success);
  }
  .to.none {
    color: var(--error);
  }
</style>
