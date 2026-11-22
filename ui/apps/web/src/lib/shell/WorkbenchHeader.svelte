<!--
  The workbench's top bar: the wordmark (Home, tagged on an instance that is
  not production), where you are and its pages, the project picker inside a
  project, ⌘K, Pending, where the gateway runs with its connect panel, Live
  updates, refresh, Redline's toggle, and the theme. What it shows is read
  by the layout and handed in.
-->
<script lang="ts">
  import { page } from '$app/state';
  import Glyph from '#lib/Glyph.svelte';
  import Icon, { type IconName } from '#lib/Icon.svelte';
  import Popover from '#lib/layer/Popover.svelte';
  import PendingPanel from '#lib/PendingPanel.svelte';
  import ThemeControl from '#lib/ThemeControl.svelte';
  import ConnectClient from '#lib/ConnectClient.svelte';
  import AddressToggle from '#lib/AddressToggle.svelte';
  import CopyButton from '#lib/CopyButton.svelte';
  import { addressView } from '#lib/address.svelte.ts';
  import { GatewayPlace, type Network } from '#lib/place.ts';
  import { ProjectBoard } from '#lib/projects.ts';
  import { getWorkbench } from '#lib/workbench.svelte.ts';
  import { Views, type Place, type Tab, type View } from '#lib/view.ts';
  import type { Environment } from '#lib/api.ts';
  import type { Stored } from '#lib/decode.ts';
  import type { RedlineDesk } from 'redline/desk';
  import type { Recipients } from '#lib/briefing.svelte.ts';

  let {
    view,
    environment,
    where,
    here,
    tab,
    inProject,
    scope,
    board,
    hostAttention,
    recording,
    error,
    updated,
    gatewayOrigin,
    redline,
    oncommand,
    onrefresh,
  }: {
    /** The page's view; null for an error or a route that is no view. */
    view: View | null;
    /** What kind of instance the gateway is, once read. */
    environment: Stored<Environment> | null;
    /** Home, or the place the page is in: the bar shows only it. */
    where: Place;
    /** The place's own entry: its name, and where that leads; null at Home. */
    here: (typeof Views.places)[number] | null;
    /** The tab lit: the page's, or the one being navigated to. */
    tab: Tab | null;
    /** The project whose pages the row shows, inside one. */
    inProject: string | null;
    /** The project a Monitor page is narrowed to, when it can be. */
    scope: string | null;
    /** What waits on the operator, across every project. */
    board: ProjectBoard;
    /** What on the host needs a look, worst first; null when nothing does. */
    hostAttention: { mark: 'failed' | 'warning'; label: string } | null;
    /** Whether the recorder lost or failed anything: its tab is marked. */
    recording: 'well' | 'attention';
    /** Why Ren's reads fail, in words; empty while they do not. */
    error: string;
    /** When the gateway was last read whole. */
    updated: number | null;
    gatewayOrigin: string;
    /** Redline's desk and where its briefs go; null without the control token. */
    redline: { desk: RedlineDesk; recipients: Recipients } | null;
    /** Open ⌘K's palette. */
    oncommand: () => void;
    onrefresh: () => void;
  } = $props();

  const workbench = getWorkbench();
  const marked = $derived(environment === null ? null : Views.instanceBadge(environment));
  // The header names where the gateway runs, not its address.
  const place = $derived(GatewayPlace.parse(gatewayOrigin));
  const shownOrigin = $derived(addressView.revealed ? place.origin : place.masked);
  const networkIcons = {
    loopback: 'monitor',
    tailnet: 'lock',
    private: 'lan',
    public: 'globe',
  } as const satisfies Record<Network, IconName>;

  /** The tab row, which scrolls sideways on a phone. */
  let tabs = $state<HTMLElement>();
  // Keep the current view's tab in sight when the row is narrower than its tabs.
  $effect(() => {
    void page.url.pathname;
    tabs
      ?.querySelector('[aria-current="page"]')
      ?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
</script>

<header class="workbench-header">
  <a
    class="brand"
    class:marked
    class:active={view === 'home'}
    href="/"
    aria-current={view === 'home' ? 'page' : undefined}
    aria-label={marked ? `Home, ${marked.tag} gateway` : 'Home'}
    title={marked?.what ?? 'Home: what needs you, what is moving, what happened'}
    ><span class="wordmark" aria-hidden="true">REN</span>{#if marked}<span
        class="instance-badge"
        aria-hidden="true">{marked.tag}</span
      >{/if}</a
  >
  <nav class="workbench-context" aria-label="Where you are" bind:this={tabs}>
    {#if here === null}
      <!-- Home leads to the three places; inside one, only it shows. -->
      {#each Views.places as each (each.place)}
        <a class="place-link" href={each.href} data-sveltekit-preload-data="hover"
          >{each.label}{#if each.place === 'system' && hostAttention}<Glyph
              mark={hostAttention.mark}
              label={hostAttention.label}
            />{/if}</a
        >
      {/each}
    {:else}
      <a
        class="place-name"
        class:active={view === Views.landing(where)}
        href={here.href}
        aria-current={view === Views.landing(where) ? 'page' : undefined}>{here.label}</a
      >
    {/if}
    {#if where === 'projects' && (inProject || view === 'scratch')}
      <button class="project-pick" popovertarget="project-picker" title="Switch project"
        ><Icon name="branch" size={13} /><strong>{inProject ?? 'Scratch'}</strong><Icon
          name="chevron"
          size={10}
        /></button
      >
      <Popover id="project-picker" label="Projects" align="left">
        <ul class="project-list">
          {#each board.active as project (project.id)}
            <li>
              <a
                href={ProjectBoard.href(project.name)}
                aria-current={project.name === inProject ? 'page' : undefined}
                ><Glyph
                  mark={board.approvals.some((waiting) => waiting.project === project.name)
                    ? 'yours'
                    : project.running
                      ? 'responding'
                      : 'idle'}
                  label={true}
                /><span>{project.name}</span></a
              >
            </li>
          {/each}
          <li>
            <a href="/scratch" aria-current={view === 'scratch' ? 'page' : undefined}
              ><Glyph mark="idle" label="Sessions that work for no project" /><span>Scratch</span
              ></a
            >
          </li>
        </ul>
      </Popover>
    {/if}
    {#each view === 'scratch' ? [] : Views.pages(where, inProject) as link (link.tab)}
      <a
        href={Views.scopedHref(link, scope)}
        data-sveltekit-preload-data="hover"
        class:active={tab === link.tab}
        aria-current={tab === link.tab ? 'page' : undefined}
        >{link.label}{#if link.tab === 'recorder' && recording === 'attention'}<Glyph
            mark="warning"
            label="Recording needs attention"
          />{/if}</a
      >
    {/each}
  </nav>
  <button class="command-open" onclick={oncommand}
    ><Icon name="search" size={13} /><span>Go to anything</span><kbd>⌘K</kbd></button
  >
  {#if where === 'projects'}
    <button
      class="pending-open"
      class:waiting={board.waiting > 0}
      popovertarget="pending-popover"
      aria-label={board.waiting ? `Pending: ${board.waiting}` : 'Pending: nothing waits on you'}
      ><Glyph mark={board.waiting ? 'yours' : 'idle'} />Pending{#if board.waiting}<span
          class="pending-count mono">{board.waiting}</span
        >{/if}</button
    >
    <Popover id="pending-popover" label="Pending" width="wide"><PendingPanel {board} /></Popover>
  {/if}
  {#if where === 'home' || where === 'system'}
    <button
      class="gateway-place mono"
      class:public={place.network === 'public'}
      popovertarget="connect-panel"
      title="Gateway on {place.node ??
        GatewayPlace.networkLabel(place.network)}, reached over {GatewayPlace.networkLabel(
        place.network,
      )}. {error ? 'The management API is not answering. ' : ''}Connect a client"
      ><Glyph
        mark={error ? 'failed' : updated ? 'passed' : 'waiting'}
      />{#if place.node && place.network !== 'loopback'}<b>{place.node}</b><span class="gateway-sep"
          >·</span
        >{/if}<Icon name={networkIcons[place.network]} size={12} /><span
        >{place.network === 'loopback'
          ? 'this machine'
          : GatewayPlace.networkLabel(place.network)}</span
      ><Icon name="chevron" size={11} /></button
    >
    <Popover id="connect-panel" label="Connect a client" width="broad">
      <header class="connect-head">
        <strong>Connect a client</strong>
        <span class="mono">{shownOrigin}</span>{#if place.maskable}<AddressToggle />{/if}<CopyButton
          text={gatewayOrigin}
          label="Copy the gateway address"
        />
      </header>
      <ConnectClient layout="list" {gatewayOrigin} />
    </Popover>
  {/if}
  <!-- Live updates and refresh: the whole workbench's, so they sit in its bar. -->
  <button
    class="bar-tool live-switch"
    class:on={workbench.automatic}
    aria-pressed={workbench.automatic}
    title={workbench.automatic
      ? 'Live updates on: pages follow new calls. Select to pause.'
      : 'Live updates paused. Select to follow new calls.'}
    onclick={() => (workbench.automatic = !workbench.automatic)}
    ><span class="live-dot" aria-hidden="true"></span>Live</button
  >
  <button
    class="bar-tool"
    onclick={onrefresh}
    disabled={workbench.loading}
    aria-label="Refresh"
    title="Refresh"
    ><span class:spinning={workbench.loading}><Icon name="refresh" size={13} /></span></button
  >
  <!-- Redline: mark what's wrong on this page for an agent to fix. A
       tool, so it sits with the bar's tools, not over the page. -->
  {#if redline}{@const { desk, recipients } = redline}<button
      class="redline-toggle"
      class:on={desk.open}
      data-redline
      aria-pressed={desk.open}
      aria-label={desk.marks.length ? `Redline: ${desk.marks.length} marks` : 'Redline'}
      onpointerenter={() => recipients.prepare()}
      onfocus={() => recipients.prepare()}
      onclick={() => desk.toggle()}
      title="Redline: mark what's wrong on this page for an agent to fix (m)"
      ><Icon name="pencil" size={13} />{#if desk.marks.length}<span class="redline-count mono"
          >{desk.marks.length}</span
        >{/if}</button
    >{/if}
  <ThemeControl />
</header>

<style>
  .workbench-header {
    display: flex;
    min-height: 44px;
    align-items: stretch;
    background: var(--surface);
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }
  /* An instance that is not production (dev, canary) says so beside the wordmark. */
  .instance-badge {
    padding: 0 4px;
    background: var(--warning);
    color: var(--on-accent);
    font: 700 9px/13px var(--mono);
    letter-spacing: 0.04em;
  }
  .brand {
    padding: 0 14px;
    display: flex;
    align-items: center;
    gap: 8px;
    border-right: 1px solid var(--line);
    border-top: 2px solid transparent;
  }
  /* The wordmark leads Home, and says so while you are there. */
  .brand.active {
    border-top-color: var(--accent);
    background: var(--canvas);
  }
  /* The wordmark, in the brand face, solid: a hairline stroke closes the
     gaps Doto leaves between its dots. */
  .wordmark {
    font: 900 22px/1 var(--brand);
    letter-spacing: 0.06em;
    color: var(--ink);
    -webkit-text-stroke: 0.4px currentColor;
  }
  .workbench-context {
    display: flex;
    align-items: stretch;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .workbench-context::-webkit-scrollbar {
    display: none;
  }
  .workbench-context > a {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    border-top: 2px solid transparent;
    color: var(--muted);
    font-size: 12.5px;
    font-weight: 500;
  }
  .workbench-context > a:hover {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .workbench-context > a.active {
    border-top-color: var(--accent);
    background: var(--canvas);
    color: var(--ink);
  }
  /* Inside a place: its name, which leads to its own home, then its pages.
     Work's home is Projects, so its name is marked there. */
  .workbench-context > a.place-name {
    border-right: 1px solid var(--line);
    color: var(--ink);
    font-size: 13.5px;
    font-weight: 600;
  }
  .workbench-context > a.place-link {
    color: var(--ink);
    font-size: 13px;
  }
  /* Inside a project: its name, which switches project, ahead of its pages. */
  .project-pick {
    display: flex;
    flex: none;
    align-items: center;
    gap: 7px;
    padding: 0 12px;
    border: 0;
    border-right: 1px solid var(--line);
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
  }
  .project-pick:hover {
    background: var(--surface-hover);
  }
  /* Its chevron points at the list it opens: down, and up while open. */
  .project-pick > :global(svg:last-child) {
    color: var(--muted);
    transform: rotate(90deg);
    transition: transform 120ms ease;
  }
  .project-pick:has(+ :global(#project-picker:popover-open)) > :global(svg:last-child) {
    transform: rotate(-90deg);
  }
  .project-list {
    margin: 0;
    padding: 4px 0;
    list-style: none;
  }
  .project-list a {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    color: var(--ink);
    font-size: 12.5px;
  }
  .project-list a:hover,
  .project-list a[aria-current='page'] {
    background: var(--surface-hover);
  }
  /* The header's right side: ⌘K and Pending first, then the gateway, Live,
     refresh, Redline, and the theme. */
  .command-open {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    align-self: center;
    width: 260px;
    height: 28px;
    margin-left: auto;
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .command-open span {
    flex: 1;
    text-align: left;
  }
  .command-open kbd {
    padding: 0 4px;
    border: 1px solid var(--control-border);
    border-bottom-width: 2px;
    border-radius: 2px;
    font-family: var(--mono);
    font-size: 10px;
  }
  .command-open:hover {
    color: var(--ink);
  }
  .pending-open {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: center;
    height: 28px;
    margin-left: 8px;
    padding: 0 10px;
    border: 1px solid var(--control-border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .pending-open.waiting {
    color: var(--ink);
    border-color: var(--warning);
  }
  .pending-count {
    padding: 0 5px;
    background: var(--warning);
    color: var(--surface);
    font-size: 11px;
  }
  /* Where the gateway runs: its node and network, never its address. */
  .gateway-place {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: stretch;
    max-width: 35%;
    padding: 0 12px;
    overflow: hidden;
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .gateway-place:hover,
  .gateway-place:focus-visible {
    color: var(--ink);
    background: var(--surface-hover);
  }
  .gateway-place :global(svg) {
    flex: none;
  }
  .gateway-place > :global(svg:last-child) {
    transform: rotate(90deg);
  }
  .gateway-place :global(.glyph) {
    margin-right: 3px;
  }
  .gateway-place b {
    color: var(--ink);
    font-weight: 500;
  }
  .gateway-sep {
    margin: 0 2px;
  }
  /* A public gateway is reachable from the internet: worth a second look. */
  .gateway-place.public :global(svg:not(:last-child)) {
    color: var(--warning);
  }
  /* The connect panel's head: opened from the gateway's place, over the page. */
  .connect-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
  }
  .connect-head strong {
    font-size: 13px;
    font-weight: 600;
  }
  .connect-head .mono {
    margin-left: auto;
    overflow: hidden;
    color: var(--muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar-tool {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border: 0;
    border-left: 1px solid var(--line);
    background: none;
    color: var(--muted);
    font-size: 12px;
    cursor: pointer;
  }
  .bar-tool:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .live-switch {
    margin-left: 8px;
  }
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--muted);
  }
  .live-switch.on {
    color: var(--ink);
  }
  .live-switch.on .live-dot {
    background: var(--success);
  }
  .redline-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 10px;
    border: 0;
    border-left: 1px solid var(--line);
    background: none;
    color: var(--muted);
    cursor: pointer;
  }
  .redline-toggle:hover {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .redline-toggle.on {
    background: var(--redline);
    color: var(--on-redline);
  }
  .redline-count {
    padding: 0 5px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 10.5px;
    font-weight: 700;
  }
  .redline-toggle.on .redline-count {
    background: var(--on-redline);
    color: var(--redline);
  }
  /* Narrower than the header's one line holds: where you are drops to a row
     of its own under the header's controls, scrolling sideways if it must. */
  @media (max-width: 1100px) {
    .workbench-header {
      flex-wrap: wrap;
    }
    .workbench-context {
      flex-basis: 100%;
      order: 10;
      min-height: 34px;
      border-top: 1px solid var(--line);
      background: var(--surface);
    }
  }
  @media (max-width: 900px) {
    .command-open {
      width: auto;
    }
    .command-open span {
      display: none;
    }
    .gateway-place {
      display: none;
    }
  }
  @media (max-width: 560px) {
    .brand {
      flex: none;
      padding: 0 8px;
      gap: 5px;
    }
    .wordmark {
      font-size: 18px;
    }
    .workbench-context > a {
      padding: 0 9px;
      font-size: 11px;
    }
  }
  @media (pointer: coarse) {
    .workbench-context > a {
      min-height: 36px;
    }
  }
</style>
