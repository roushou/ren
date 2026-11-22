<!-- A `Chip`: its label, coloured by its tone, explained on hover. -->
<script lang="ts">
  import type { Chip } from './display.ts';

  let { chip }: { chip: Chip } = $props();
</script>

<span class="chip {chip.tone}" title={chip.title}>{chip.label}</span>
