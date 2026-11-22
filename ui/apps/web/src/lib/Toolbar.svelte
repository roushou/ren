<!--
  The bar under the header: what the view looks at and the controls that
  change it (its scope), drawn only for a page that has one. Live updates
  and refresh are the header's. That the shown data may be out of date is
  the bottom edge's to say. Each page renders its own, first in its
  markup, so the server draws it complete with the page instead of the
  layout drawing it before the page has said what goes in it.
-->
<script lang="ts">
  import { onMount, tick, type Snippet } from 'svelte';
  import { page } from '$app/state';
  import Icon from './Icon.svelte';
  import { Views } from './view.ts';

  let { scope }: { scope?: Snippet } = $props();
  const view = $derived(Views.of(page.route.id));
  const title = $derived(page.error ? 'Page unavailable' : view ? Views.title(view) : 'Ren');

  /** When the scope does not fit on one line it folds behind a button: by
   * measurement here, and on a phone by its width in CSS. */
  let scopeOpen = $state(false);
  let folded = $state(false);
  let bar = $state<HTMLElement>();
  let scopeElement = $state<HTMLElement>();
  /** The scope's width on one line, measured while it is shown inline. */
  let natural = 0;
  function fit() {
    if (!bar || !scopeElement) return;
    if (!folded) {
      // The content's own width on one line: laid out for a moment at its
      // max-content width, so nothing inside is shrunk while it is read.
      // (Set from script; the CSP refuses only inline style in markup.)
      const before = scopeElement.style.cssText;
      scopeElement.style.flex = 'none';
      scopeElement.style.width = 'max-content';
      natural = scopeElement.offsetWidth;
      scopeElement.style.cssText = before;
    }
    const padding = 24 + 12;
    // A phone folds by its width in CSS; this measures every other width.
    folded = natural > bar.clientWidth - padding;
    // Unfolded, the panel has nothing to open; a phone keeps it folded, so
    // what the operator opened there stays open.
    if (!folded && !window.matchMedia('(max-width: 760px)').matches) scopeOpen = false;
  }
  onMount(() => {
    void tick().then(fit);
    const observer = new ResizeObserver(() => fit());
    if (bar) observer.observe(bar);
    // The scope grows as the page's data arrives (a recording's name, its
    // figures): measure again, unfolded, whenever it changes.
    const changes = new MutationObserver(() => {
      if (folded) return;
      fit();
    });
    if (scopeElement)
      changes.observe(scopeElement, { childList: true, subtree: true, characterData: true });
    return () => {
      observer.disconnect();
      changes.disconnect();
    };
  });
</script>

<!-- The nav tab names the view; a page with nothing of its own to scope draws no bar. -->
<h1 class="sr-only">{title}</h1>
{#if scope}
  <div class="workbench-toolbar" class:folded class:scope-open={scopeOpen} bind:this={bar}>
    <button
      class="button small scope-toggle"
      aria-expanded={scopeOpen}
      aria-controls="scope"
      onclick={() => (scopeOpen = !scopeOpen)}><Icon name="filter" size={13} /> Scope</button
    >
    <div class="scope" id="scope" bind:this={scopeElement}>{@render scope()}</div>
  </div>
{/if}
