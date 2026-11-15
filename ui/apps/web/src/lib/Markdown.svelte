<!--
  Markdown, rendered as elements only (`markdown.ts` says how it is made
  inert): no HTML is injected, links show their address without being
  clickable, and images are named, not loaded.
-->
<script lang="ts">
  import { MarkdownTree } from './markdown.ts';
  import MarkdownBlocks from './MarkdownBlocks.svelte';

  let { text }: { text: string } = $props();

  const blocks = $derived(MarkdownTree.parse(text));
</script>

<div class="markdown"><MarkdownBlocks {blocks} /></div>
