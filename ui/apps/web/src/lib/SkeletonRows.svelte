<!--
  A table's rows while its data is on its way: one cell per declared column,
  with the column's own class and alignment, so the outline is the table.
-->
<script lang="ts">
  import Skeleton from './Skeleton.svelte';
  import type { Column } from './columns.ts';

  let { columns, rows = 6 }: { columns: readonly Column[]; rows?: number } = $props();

  /** Text widths, varied so the rows read as text rather than stripes. */
  const widths = [
    '72%',
    '54%',
    '86%',
    '63%',
    '47%',
    '80%',
    '58%',
    '69%',
    '51%',
    '84%',
    '60%',
    '75%',
  ];
</script>

{#each { length: rows }, row (row)}
  <tr class="skeleton-row">
    {#each columns as column, index (index)}
      <td class={column.class} class:numeric={column.numeric}
        ><Skeleton
          width={column.numeric ? '3.5em' : widths[(row + index * 5) % widths.length]}
        /></td
      >
    {/each}
  </tr>
{/each}
