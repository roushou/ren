<!--
  How to start a session through this gateway: a choice of client and the
  `tgv run` command for it, ready to copy.

  `inline` is one line inside a panel; `box` is a bordered box with the
  clients as tabs over the command.
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import CopyButton from './CopyButton.svelte';
  import Segmented, { type SegmentedOption } from './Segmented.svelte';
  import { harnessLabel } from './activity.ts';
  import { LaunchCommand, launchClients, type LaunchClient } from './launch.ts';

  let {
    gatewayOrigin,
    layout,
  }: {
    /** The gateway clients reach, as an origin (`page.data.gatewayUrl`). */
    gatewayOrigin: string;
    layout: 'inline' | 'box';
  } = $props();

  const clients: SegmentedOption<LaunchClient>[] = launchClients.map((client) => ({
    value: client.run,
    label: harnessLabel(client.harness),
  }));
  let client = $state<LaunchClient>('claude');
  const command = $derived(LaunchCommand.for(client, gatewayOrigin).text);
</script>

{#if layout === 'inline'}
  <section class="connect-inline" aria-label="Connect a client">
    <Icon name="terminal" size={14} />
    <span class="muted">Start a session through TGV:</span>
    <Segmented label="Client to connect" options={clients} bind:value={client} />
    <code>{command}</code><CopyButton text={command} label="Copy launch command" />
  </section>
{:else}
  <section class="connect-box" aria-label="Connect a client">
    <Segmented label="Client to connect" look="tabs" options={clients} bind:value={client} />
    <div class="command-line">
      <Icon name="terminal" size={16} /><code>{command}</code><CopyButton
        text={command}
        label="Copy launch command"
      />
    </div>
  </section>
{/if}
