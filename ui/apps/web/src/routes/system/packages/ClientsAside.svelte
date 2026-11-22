<!--
  Each client's sign-in on the daemon's host, beside the packages: signed
  out, it offers to sign in where Ren can; otherwise to check again.
-->
<script lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import { SignIns } from '#lib/signIn.ts';
  import { Runs, type Drivable, type RunClient } from '#lib/runs.ts';
  import { harnessLabel } from '#lib/activity.ts';

  let {
    clients,
    checking,
    oncheck,
    onsignin,
  }: {
    clients: RunClient[];
    /** The client whose sign-in is being checked, if any. */
    checking: Drivable | null;
    oncheck: (harness: Drivable) => void;
    onsignin: (harness: Drivable) => void;
  } = $props();
</script>

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
          <button class="button small" onclick={() => onsignin(harness)}>Sign in</button>
        {:else}
          <button
            class="button small"
            disabled={checking === harness}
            onclick={() => oncheck(harness)}
            >{checking === harness ? 'Checking…' : 'Check again'}</button
          >
        {/if}
      {/if}
    </div>
  {/each}
</aside>

<style>
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
    .clients {
      border-top: 1px solid var(--line);
      border-left: 0;
    }
  }
</style>
