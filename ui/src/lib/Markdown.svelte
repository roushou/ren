<script lang="ts">
  // Renders parsed Markdown as elements only: no HTML is injected, links show
  // their address without being clickable, and images are named, not loaded.
  import { parseMarkdown, type Block, type Inline } from './markdown.ts';
  let { text }: { text: string } = $props();
  const blocks = $derived(parseMarkdown(text));
</script>

{#snippet inlines(
  nodes: Inline[],
)}{#each nodes as node}{#if node.kind === 'text'}{node.text}{:else if node.kind === 'code'}<code
        >{node.text}</code
      >{:else if node.kind === 'strong'}<strong>{@render inlines(node.children)}</strong
      >{:else if node.kind === 'em'}<em>{@render inlines(node.children)}</em
      >{:else if node.kind === 'strike'}<s>{@render inlines(node.children)}</s
      >{:else if node.kind === 'link'}<span class="md-link" title={node.href}
        >{@render inlines(node.children)}<span class="md-href">{node.href}</span></span
      >{:else if node.kind === 'image'}<span class="md-image" title={node.href}
        >image: {node.alt || node.href}</span
      >{/if}{/each}{/snippet}

{#snippet render(nodes: Block[])}
  {#each nodes as block}
    {#if block.kind === 'heading'}
      <div class="md-heading md-h{block.level}" role="heading" aria-level={block.level + 3}>
        {@render inlines(block.children)}
      </div>
    {:else if block.kind === 'paragraph'}
      <p>{@render inlines(block.children)}</p>
    {:else if block.kind === 'code'}
      <div class="md-code">
        {#if block.lang}<span class="md-lang">{block.lang}</span>{/if}
        <pre>{block.text}</pre>
      </div>
    {:else if block.kind === 'quote'}
      <blockquote>{@render render(block.children)}</blockquote>
    {:else if block.kind === 'list'}
      {#if block.ordered}<ol start={block.start}>
          {#each block.items as item}<li>{@render render(item)}</li>{/each}
        </ol>{:else}<ul>
          {#each block.items as item}<li>{@render render(item)}</li>{/each}
        </ul>{/if}
    {:else if block.kind === 'tagged'}
      <details class="md-tagged">
        <summary><code>{block.name}</code></summary>
        {@render render(block.children)}
      </details>
    {:else if block.kind === 'rule'}
      <hr />
    {:else if block.kind === 'table'}
      <div class="md-table">
        <table>
          <thead
            ><tr
              >{#each block.header as cell}<th>{@render inlines(cell)}</th>{/each}</tr
            ></thead
          >
          <tbody>
            {#each block.rows as row}<tr
                >{#each row as cell}<td>{@render inlines(cell)}</td>{/each}</tr
              >{/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/each}
{/snippet}

<div class="markdown">{@render render(blocks)}</div>
