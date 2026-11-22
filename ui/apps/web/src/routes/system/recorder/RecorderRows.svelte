<!--
  A block's figures as rows: each one's name, and its value on the right.
-->
<script lang="ts">
  let { rows }: { rows: { label: string; value: string }[] } = $props();
</script>

<dl class="rows">
  {#each rows as row (row.label)}<div>
      <dt>{row.label}</dt>
      <dd class="mono">{row.value}</dd>
    </div>{/each}
</dl>

<style>
  dl {
    margin: 0;
  }
  dd {
    margin: 0;
    font-family: var(--mono);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    text-align: right;
  }
  .rows > div {
    display: flex;
    justify-content: space-between;
    padding: 3px 0;
    font-size: 12.5px;
  }
  .rows dt {
    color: var(--ink);
  }
</style>
