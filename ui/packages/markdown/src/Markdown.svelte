<!--
  Markdown, rendered as elements only (`tree.ts` says how it is made
  inert): no HTML is injected, links show their address without being
  clickable, and images are named, not loaded.
-->
<script lang="ts">
  import { Markdown } from './tree.ts';
  import './markdown.css';
  import MarkdownBlocks from './MarkdownBlocks.svelte';

  let { text }: { text: string } = $props();

  const markdown = $derived(Markdown.parse(text));
</script>

<div class="markdown"><MarkdownBlocks blocks={markdown.blocks} /></div>
