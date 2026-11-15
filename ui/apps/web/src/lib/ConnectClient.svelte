<!--
  How to start a session through this gateway: a choice of client and the
  `ren run` command for it, ready to copy.

  `list` is every client's command, one per row, for the header's connect
  panel; `box` is a bordered box with the clients as tabs over the command.
  Commands show the address masked unless it is revealed (`addressView`);
  copying always copies the real command.
-->
<script lang="ts">
  import Icon from './Icon.svelte';
  import CopyButton from './CopyButton.svelte';
  import Segmented, { type SegmentedOption } from './Segmented.svelte';
  import { harnessLabel } from './activity.ts';
  import { LaunchCommand, launchClients, type LaunchClient } from './launch.ts';
  import AddressToggle from './AddressToggle.svelte';
  import { addressView } from './address.svelte.ts';
  import { GatewayPlace } from './place.ts';

  let {
    gatewayOrigin,
    layout,
  }: {
    /** The gateway clients reach, as an origin (`page.data.gatewayUrl`). */
    gatewayOrigin: string;
    layout: 'list' | 'box';
  } = $props();

  const clients: SegmentedOption<LaunchClient>[] = launchClients.map((client) => ({
    value: client.run,
    label: harnessLabel(client.harness),
  }));
  let client = $state<LaunchClient>('claude');
  const command = $derived(LaunchCommand.for(client, gatewayOrigin).text);
  const place = $derived(GatewayPlace.parse(gatewayOrigin));
  const shown = (text: string) => (addressView.revealed ? text : place.mask(text));
</script>

{#if layout === 'list'}
  <ul class="connect-list" aria-label="Launch commands">
    {#each launchClients as each (each.run)}
      {@const text = LaunchCommand.for(each.run, gatewayOrigin).text}
      <li>
        <span>{harnessLabel(each.harness)}</span><code>{shown(text)}</code><CopyButton
          {text}
          label="Copy the {harnessLabel(each.harness)} command"
        />
      </li>
    {/each}
  </ul>
{:else}
  <section class="connect-box" aria-label="Connect a client">
    <Segmented label="Client to connect" look="tabs" options={clients} bind:value={client} />
    <div class="command-line">
      <Icon name="terminal" size={16} /><code>{shown(command)}</code
      >{#if place.maskable}<AddressToggle />{/if}<CopyButton
        text={command}
        label="Copy launch command"
      />
    </div>
  </section>
{/if}
