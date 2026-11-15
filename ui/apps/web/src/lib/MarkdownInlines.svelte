<!--
  Markdown spans, each as its element; nested spans render through this
  component again. A link shows its address and is not followed; an image
  is named and not loaded.
-->
<script lang="ts">
  import type { Inline } from './markdown.ts';
  import MarkdownInlines from './MarkdownInlines.svelte';

  let { inlines }: { inlines: readonly Inline[] } = $props();
</script>

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
    <span class="md-link" title={inline.href}
      ><MarkdownInlines inlines={inline.children} /><span class="md-href">{inline.href}</span></span
    >
  {:else if inline.kind === 'image'}
    <span class="md-image" title={inline.href}>image: {inline.alt || inline.href}</span>
  {:else if inline.kind === 'break'}
    <br />
  {:else if inline.kind === 'footnote'}
    <sup class="md-footnote-ref">{inline.label}</sup>
  {/if}
{/each}
