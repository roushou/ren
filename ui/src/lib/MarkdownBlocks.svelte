<!--
  Markdown blocks, each as its element; nested blocks render through this
  component again, and spans through `MarkdownInlines`. Headings are
  levelled below the page's own, so a reply never outranks its view.
-->
<script lang="ts">
  import type { Block } from './markdown.ts';
  import MarkdownBlocks from './MarkdownBlocks.svelte';
  import MarkdownInlines from './MarkdownInlines.svelte';

  let { blocks }: { blocks: readonly Block[] } = $props();
</script>

{#each blocks as block, index (index)}
  {#if block.kind === 'heading'}
    <div class="md-heading md-h{block.level}" role="heading" aria-level={block.level + 3}>
      <MarkdownInlines inlines={block.children} />
    </div>
  {:else if block.kind === 'paragraph'}
    <p><MarkdownInlines inlines={block.children} /></p>
  {:else if block.kind === 'code'}
    <div class="md-code">
      {#if block.lang}<span class="md-lang">{block.lang}</span>{/if}
      <pre>{block.text}</pre>
    </div>
  {:else if block.kind === 'quote'}
    <blockquote><MarkdownBlocks blocks={block.children} /></blockquote>
  {:else if block.kind === 'list'}
    <svelte:element this={block.ordered ? 'ol' : 'ul'} start={block.ordered ? block.start : null}>
      {#each block.items as item, at (at)}
        <li class:md-task={item.checked !== null}>
          {#if item.checked !== null}
            <input type="checkbox" checked={item.checked} disabled aria-label="Task" />
          {/if}
          <MarkdownBlocks blocks={item.children} />
        </li>
      {/each}
    </svelte:element>
  {:else if block.kind === 'rule'}
    <hr />
  {:else if block.kind === 'table'}
    <div class="md-table">
      <table>
        <thead>
          <tr>
            {#each block.header as cell, column (column)}
              <th style:text-align={block.align[column]}><MarkdownInlines inlines={cell} /></th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each block.rows as row, at (at)}
            <tr>
              {#each row as cell, column (column)}
                <td style:text-align={block.align[column]}><MarkdownInlines inlines={cell} /></td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if block.kind === 'footnote'}
    <div class="md-footnote">
      <span class="md-footnote-label">{block.label}</span>
      <div><MarkdownBlocks blocks={block.children} /></div>
    </div>
  {:else if block.kind === 'tagged'}
    <details class="md-tagged">
      <summary><code>{block.name}</code></summary>
      <MarkdownBlocks blocks={block.children} />
    </details>
  {/if}
{/each}
