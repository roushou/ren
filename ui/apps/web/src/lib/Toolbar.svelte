<!--
  The bar under the header: what the view looks at and the controls that
  change it (its scope), then live updates and refresh, then any notice that
  the shown data may be out of date. Each page renders its own, first in its
  markup, so the server draws it complete with the page instead of the
  layout drawing it before the page has said what goes in it.
-->
<script lang="ts">
  import { onMount, tick, type Snippet } from 'svelte';
  import { page } from '$app/state';
  import Icon from './Icon.svelte';
  import { getWorkbench } from './workbench.svelte.ts';
  import { Views } from './view.ts';

  let { scope }: { scope?: Snippet } = $props();
  const workbench = getWorkbench();
  const controls = workbench.controls;
  const title = $derived(page.error ? 'Page unavailable' : Views.title(Views.of(page.route.id)));
  const loading = $derived(controls?.loading() ?? false);
  const problem = $derived(controls?.problem() ?? '');

  /** When the scope does not fit on one line it folds behind a button. */
  let scopeOpen = $state(false);
  let folded = $state(false);
  let bar = $state<HTMLElement>();
  let scopeElement = $state<HTMLElement>();
  /** The scope's width on one line, measured while it is shown inline. */
  let natural = 0;
  function fit() {
    if (!bar || !scopeElement) return;
    if (!folded) {
      // The content's own width: its groups and the 24px between each.
      const groups = [...scopeElement.children] as HTMLElement[];
      natural =
        groups.reduce((sum, group) => sum + group.offsetWidth, 0) +
        Math.max(0, groups.length - 1) * 24;
    }
    const actions = bar.querySelector<HTMLElement>('.page-actions')?.offsetWidth ?? 0;
    const padding = 24 + 12;
    folded = !!scope && natural > bar.clientWidth - actions - padding;
    if (!folded) scopeOpen = false;
  }
  onMount(() => {
    void tick().then(fit);
    const observer = new ResizeObserver(() => fit());
    if (bar) observer.observe(bar);
    return () => observer.disconnect();
  });
</script>

<div class="workbench-toolbar" class:folded class:scope-open={scopeOpen} bind:this={bar}>
  <!-- The nav tab names the view; the bar shows its scope. -->
  <h1 class="sr-only">{title}</h1>
  {#if scope && folded}<button
      class="button small scope-toggle"
      aria-expanded={scopeOpen}
      aria-controls="scope"
      onclick={() => (scopeOpen = !scopeOpen)}><Icon name="filter" size={13} /> Scope</button
    >{/if}
  <div class="scope" id="scope" bind:this={scopeElement}>
    {#if scope}{@render scope()}{:else}<span class="scope-title">{title}</span>{/if}
  </div>
  <div class="page-actions">
    <label class="toggle-control"
      ><input type="checkbox" bind:checked={workbench.automatic} /><span class="switch"></span><span
        >Live updates</span
      ></label
    >
    <button
      class="icon-button"
      onclick={() => controls?.refresh()}
      disabled={loading}
      aria-label="Refresh workspace"
      title="Refresh workspace"
      ><span class:spinning={loading}><Icon name="refresh" size={14} /></span></button
    >
  </div>
</div>
{#if problem}<div class="notice danger" role="alert">
    <Icon name="alert" size={14} /><span>{problem} Displayed data may be out of date.</span><button
      class="text-button"
      onclick={() => controls?.refresh()}>Retry</button
    >
  </div>{/if}
