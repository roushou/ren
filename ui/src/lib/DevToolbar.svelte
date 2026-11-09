<!--
  The dev slot's traffic controls, floating over the workbench. Shown only on a
  development gateway whose UI server has the slot's traffic controls; changes
  apply at once. Collapses to a small tab, remembered per browser.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { DevTraffic, PACES, SINGLE_REPLIES, type TrafficSettings } from './devtraffic.ts';
  import Icon from './Icon.svelte';

  const traffic = new DevTraffic();
  const storageKey = 'tgv.devToolbar';
  let settings = $state<TrafficSettings | null>(null);
  let sent = $state(0);
  let inFlight = $state(0);
  let error = $state('');
  let open = $state(true);
  let mix = $state(false);
  let saving: ReturnType<typeof setTimeout> | undefined;

  const groups = [
    { key: 'replies', title: 'Replies' },
    { key: 'harnesses', title: 'Harnesses' },
    { key: 'senders', title: 'Callers' },
  ] as const;

  async function refresh(first: boolean) {
    try {
      const status = await traffic.status();
      sent = status.sent;
      inFlight = status.in_flight;
      if (first || !settings) settings = status.settings;
      error = '';
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function save() {
    clearTimeout(saving);
    saving = setTimeout(async () => {
      if (!settings) return;
      try {
        await traffic.save($state.snapshot(settings));
        error = '';
      } catch (cause) {
        error = cause instanceof Error ? cause.message : String(cause);
      }
    }, 250);
  }

  async function send(count: number, reply?: string) {
    try {
      await traffic.burst(count, reply);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function toggle(next: boolean) {
    open = next;
    try {
      localStorage.setItem(storageKey, next ? 'open' : 'closed');
    } catch {
      // A browser without storage keeps the toolbar open each visit.
    }
  }

  onMount(() => {
    try {
      open = localStorage.getItem(storageKey) !== 'closed';
    } catch {
      open = true;
    }
    refresh(true);
    const timer = setInterval(() => refresh(false), 2000);
    return () => clearInterval(timer);
  });
</script>

{#if !open}
  <button
    class="dev-toolbar-tab"
    type="button"
    onclick={() => toggle(true)}
    title="Show the dev traffic controls"
    >Traffic{#if settings}<span class="mono"
        >{settings.flow === 'paused' ? 'paused' : `${settings.rate}/min`}</span
      >{/if}</button
  >
{:else}
  <section class="dev-toolbar" aria-label="Dev traffic controls">
    {#if mix && settings}
      <div class="dev-toolbar-mix">
        {#each groups as group (group.key)}
          <div class="dev-toolbar-group" role="group" aria-label={group.title}>
            <h3>{group.title}</h3>
            {#each Object.keys(settings[group.key]) as name (name)}
              <label
                ><span>{DevTraffic.label(name)}</span><input
                  type="range"
                  min="0"
                  max="100"
                  step="1"
                  bind:value={settings[group.key][name]}
                  oninput={save}
                /><output class="mono">{settings[group.key][name]}</output></label
              >
            {/each}
          </div>
        {/each}
        <div class="dev-toolbar-group dev-toolbar-send" role="group" aria-label="Send one">
          <h3>Send one</h3>
          <div class="dev-toolbar-singles">
            {#each SINGLE_REPLIES as single (single.reply)}
              <button type="button" class="button small" onclick={() => send(1, single.reply)}
                >{single.label}</button
              >
            {/each}
          </div>
        </div>
      </div>
    {/if}
    <div class="dev-toolbar-bar">
      <span class="dev-toolbar-title">Traffic</span>
      {#if settings}
        <button
          type="button"
          class="button small"
          aria-pressed={settings.flow === 'running'}
          onclick={() => {
            if (!settings) return;
            settings.flow = settings.flow === 'paused' ? 'running' : 'paused';
            save();
          }}>{settings.flow === 'paused' ? 'Resume' : 'Pause'}</button
        >
        <label class="dev-toolbar-rate"
          ><input
            type="range"
            min="0"
            max={DevTraffic.MAX_RATE}
            step="1"
            aria-label="Calls a minute"
            bind:value={settings.rate}
            oninput={save}
          /><output class="mono">{settings.rate}/min</output></label
        >
        <span class="dev-toolbar-pace" role="group" aria-label="Reply pace"
          >{#each PACES as pace (pace)}<button
              type="button"
              aria-pressed={settings.pace === pace}
              onclick={() => {
                if (!settings) return;
                settings.pace = pace;
                save();
              }}>{pace}</button
            >{/each}</span
        >
        <button type="button" class="button small" onclick={() => send(20)}>Burst 20</button>
        <button type="button" class="button small" aria-expanded={mix} onclick={() => (mix = !mix)}
          >Mix</button
        >
        <span class="mono dev-toolbar-counts">{sent} sent · {inFlight} in flight</span>
      {/if}
      {#if error}<span class="dev-toolbar-error" role="alert">{error}</span>{/if}
      <button
        type="button"
        class="dev-toolbar-close"
        aria-label="Hide the dev traffic controls"
        onclick={() => toggle(false)}><Icon name="close" size={13} /></button
      >
    </div>
  </section>
{/if}
