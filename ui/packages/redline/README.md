# redline

Mark elements on a page, note what's wrong with each, and get a Markdown brief
an agent can act on. Svelte 5. It sends nothing itself: where a brief goes is
the host's.

## Contract

| Part                    | What it is                                                                                                                                                                                                                    |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Redline` (`redline`)   | The pure core. `selector` names an element from its nearest `aria-label`; `brief(page, marks, framing)` makes the Markdown brief from the noted marks, with the host's `Framing` (lines about the page, a closing paragraph). |
| `RedlineDesk` (`/desk`) | One page view's state: whether marking is on, the marks, the note being written.                                                                                                                                              |
| `RedlineLayer.svelte`   | Marks the page while the desk is open: outlines the element under the pointer, holds the page still, takes a click as a mark, and pins each mark. `m` toggles, `Esc` leaves.                                                  |
| `RedlinePanel.svelte`   | The floating pane: the marks and the folded brief. Props: `desk`, `brief`, `keep` (the `localStorage` key for its place), and the snippets `preview` (the brief as the host renders it) and `footer` (where it goes).         |
| `redline.css`           | The theme, imported by both components: `--redline-*` custom properties with defaults at no specificity, which a host overrides on `:root`.                                                                                   |

Everything Redline draws carries `data-redline`; the layer leaves clicks on
such elements alone, so a host's controls inside the pane keep working. Source
locations come from Svelte's development build (`__svelte_meta`); production
builds have none.

## Use

```svelte
<script lang="ts">
  import { Redline } from 'redline';
  import { RedlineDesk } from 'redline/desk';
  import RedlineLayer from 'redline/RedlineLayer.svelte';
  import RedlinePanel from 'redline/RedlinePanel.svelte';

  const desk = new RedlineDesk();
  const brief = $derived(
    Redline.brief(
      {
        path: location.pathname,
        title: document.title,
        width: innerWidth,
        height: innerHeight,
        theme: 'light',
      },
      desk.marks,
      { about: [], closing: 'Answer what the notes ask.' },
    ),
  );
</script>

<RedlineLayer {desk} />
{#if desk.open}
  <RedlinePanel {desk} {brief}>
    {#snippet footer()}
      <button onclick={() => navigator.clipboard.writeText(brief)}>Copy</button>
    {/snippet}
  </RedlinePanel>
{/if}
```

## Checks

`bun run check` and `bun run test` here. `tests/boundary.test.ts` keeps the
sources importing only each other and Svelte.
