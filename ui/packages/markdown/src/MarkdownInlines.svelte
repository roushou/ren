<!--
  Markdown spans, each as its element; nested spans render through this
  component again. A web link opens in a new tab, any other is only shown,
  and a link whose text is not its address shows the address too; an image
  is named and not loaded.
-->
<script lang="ts">
  import type { Inline, LinkAddress } from './tree.ts';
  import MarkdownInlines from './MarkdownInlines.svelte';

  let { inlines }: { inlines: readonly Inline[] } = $props();
</script>

<!-- A link's text, then its address when the text is not it. -->
{#snippet label(children: readonly Inline[], address: LinkAddress, shown: string)}
  <MarkdownInlines inlines={children} />{#if address === 'beside'}<span class="md-href"
      >{shown}</span
    >{/if}
{/snippet}

{#each inlines as inline, index (index)}
  {#if inline.kind === 'text'}
    {inline.text}
  {:else if inline.kind === 'code'}
    <code>{inline.text}</code>
  {:else if inline.kind === 'strong'}
    <strong><MarkdownInlines inlines={inline.children} /></strong>
  {:else if inline.kind === 'em'}
    <em><MarkdownInlines inlines={inline.children} /></em>
  {:else if inline.kind === 'strike'}
    <s><MarkdownInlines inlines={inline.children} /></s>
  {:else if inline.kind === 'link'}
    {@const shown = inline.target.kind === 'web' ? inline.target.url : inline.target.href}
    {#if inline.target.kind === 'web'}
      <a
        class="md-link"
        href={inline.target.url}
        target="_blank"
        rel="noopener noreferrer"
        title={shown}>{@render label(inline.children, inline.address, shown)}</a
      >
    {:else}
      <span class="md-link inert" title={shown}
        >{@render label(inline.children, inline.address, shown)}</span
      >
    {/if}
  {:else if inline.kind === 'image'}
    <span class="md-image" title={inline.href}>image: {inline.alt || inline.href}</span>
  {:else if inline.kind === 'break'}
    <br />
  {:else if inline.kind === 'footnote'}
    <sup class="md-footnote-ref">{inline.label}</sup>
  {/if}
{/each}
