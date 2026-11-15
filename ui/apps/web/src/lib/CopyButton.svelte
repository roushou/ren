<script lang="ts">
  import { onDestroy } from 'svelte';
  import Icon from './Icon.svelte';
  let { text, label = 'Copy' }: { text: string; label?: string } = $props();
  let result = $state('');
  let timer: ReturnType<typeof setTimeout>;
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      result = 'Copied';
    } catch {
      result = 'Could not copy. Select the text to copy manually.';
    }
    clearTimeout(timer);
    timer = setTimeout(() => (result = ''), 2500);
  }
  onDestroy(() => clearTimeout(timer));
</script>

<button class="icon-button" onclick={copy} aria-label={label} title={result || label}
  ><Icon name={result === 'Copied' ? 'check' : 'copy'} /></button
>
<span class="sr-only" role="status">{result}</span>
