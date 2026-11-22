<!--
  The packages, grouped by kind: each package's standing, where runs get it,
  the clients that need it, the newest generations beside it as columns, and
  the one change its button stages. While the listing is on its way it is an
  outline: the catalog's groups with bars where packages go.
-->
<script module lang="ts">
  import Glyph from '#lib/Glyph.svelte';
  import {
    KINDS,
    Origins,
    PackageGroups,
    PackageHistory,
    RowActions,
    Supplies,
    GenerationStates,
    type Applying,
    type Effect,
    type Generation,
    type KindGroup,
    type Package,
    type PackageId,
    type PackageListing,
    type PackagePlan,
  } from '#lib/packages.ts';

  export { dot };

  /** What the table shows: the outline while the listing is on its way, or
   * the listing's packages with the plan staged over them. */
  export type PackageRows =
    | { kind: 'outline' }
    | {
        kind: 'listing';
        listing: PackageListing;
        groups: KindGroup[];
        plan: PackagePlan;
        /** Each staged package's change, as runs see it. */
        staged: ReadonlyMap<PackageId, Effect>;
        selected: PackageId | null;
        applying: Applying;
      };

  /** The outline's groups: the catalog's kinds, with rows varied in width. */
  const OUTLINE = KINDS.map((kind, index) => ({
    kind,
    title: PackageGroups.title(kind),
    rows: [3, 4, 3][index] ?? 3,
  }));
  const OUTLINE_WIDTHS = ['62%', '48%', '71%', '55%'] as const;
</script>

<script lang="ts">
  import Icon from '#lib/Icon.svelte';
  import Skeleton from '#lib/Skeleton.svelte';
  import { harnessLabel } from '#lib/activity.ts';

  let {
    rows,
    onselect = () => {},
    ontoggle = () => {},
    onrestore = () => {},
  }: {
    rows: PackageRows;
    onselect?: (id: PackageId) => void;
    ontoggle?: (pkg: Package) => void;
    onrestore?: (generation: Generation) => void;
  } = $props();
</script>

{#snippet dot(pkg: Package)}
  {@const standing = Supplies.mark(pkg)}<Glyph mark={standing.mark} label={standing.word} />
{/snippet}

{#if rows.kind === 'outline'}
  <!-- The listing on its way: the catalog's groups with rows where packages
       go. -->
  <section class="packages-main" aria-label="Packages" aria-busy="true">
    <table class="packages-table">
      <colgroup>
        <col class="c-name" />
        <col class="c-version" />
        <col />
        <col class="c-action" />
      </colgroup>
      {#each OUTLINE as group (group.kind)}
        <tbody>
          <tr class="packages-group">
            <th colspan="4" scope="rowgroup">{group.title}</th>
          </tr>
          {#each { length: group.rows }, row (row)}
            <tr aria-hidden="true">
              <td class="mono name"
                ><Glyph mark="idle" /><Skeleton
                  width={OUTLINE_WIDTHS[row % OUTLINE_WIDTHS.length]}
                /></td
              >
              <td class="mono"><Skeleton width="4.5em" /></td>
              <td class="source"
                ><Skeleton width={OUTLINE_WIDTHS[(row + 2) % OUTLINE_WIDTHS.length]} /></td
              >
              <td class="end"></td>
            </tr>
          {/each}
        </tbody>
      {/each}
    </table>
  </section>
{:else}
  {@const { listing, groups, plan, staged, selected, applying } = rows}
  <!-- The newest generations, as columns beside the packages. -->
  {@const history = new PackageHistory(listing)}
  <section class="packages-main" aria-label="Packages">
    <table class="packages-table">
      <colgroup>
        <col class="c-name" />
        <col class="c-origin" />
        <col class="c-version" />
        <col />
        <col class="c-needed" />
        {#each history.generations as generation (generation.generation)}<col
            class="c-gen"
          />{/each}
        <col class="c-action" />
      </colgroup>
      <thead>
        <tr>
          <th>Package</th>
          <th>Origin</th>
          <th>Version</th>
          <th>Path</th>
          <th>Needed by</th>
          {#each history.generations as generation (generation.generation)}
            <th class="gen-head" class:current={generation.state === 'current'}
              ><span class="mono">#{generation.generation}</span
              >{#if generation.state === 'current'}<Glyph
                  mark="passed"
                  label={GenerationStates.label(generation.state)}
                />{:else if generation.state === 'in_use'}<Glyph
                  mark="responding"
                  label="In use by runs"
                />{/if}{#if generation.state !== 'current' && listing.platform}<button
                  class="icon-button restore"
                  title="Restore generation {generation.generation}…"
                  aria-label="Restore generation {generation.generation}"
                  disabled={applying.kind === 'applying'}
                  onclick={() => onrestore(generation.generation)}
                  ><Icon name="refresh" size={11} /></button
                >{/if}</th
            >
          {/each}
          <th></th>
        </tr>
      </thead>
      {#each groups as group (group.kind)}
        <tbody>
          <tr class="packages-group">
            <th colspan={6 + history.generations.length} scope="rowgroup"
              >{group.title} <span class="count">{group.packages.length}</span></th
            >
          </tr>
          {#each group.packages as pkg (pkg.id)}
            {@const label = RowActions.label(pkg, plan)}
            {@const supply = Supplies.of(pkg)}
            {@const change = staged.get(pkg.id)}
            <tr
              class="packages-row"
              class:selected={selected === pkg.id}
              class:staged={change !== undefined}
              onclick={() => onselect(pkg.id)}
            >
              <td class="mono name"
                >{@render dot(pkg)}{pkg.id}{#if pkg.program !== pkg.id}<span class="muted alias"
                    >{pkg.program}</span
                  >{/if}</td
              >
              <td class="mono muted">{Origins.label(pkg.origin)}</td>
              <td class="mono"
                >{#if change}<s class="muted">{change.from ?? '—'}</s><span class="arrow">→</span
                  ><span class="to {change.toKind}">{change.to ?? '—'}</span
                  >{:else if supply.kind === 'managed'}{supply.version}{:else if supply.kind === 'system'}{#if supply.system.version}{supply
                      .system.version}{:else}<span
                      class="unknown"
                      title={supply.system.unread ?? ''}>?</span
                    >{/if}{:else}<span class="muted">—</span>{/if}</td
              >
              <td class="mono muted path"
                >{#if supply.kind === 'system'}{supply.system
                    .path}{:else if supply.kind === 'managed' && supply.shadows}<span
                    class="shadowed"
                    title="Runs find Ren’s first">{supply.shadows.path}</span
                  >{:else if Supplies.tone(pkg) === 'missing'}not found{:else}—{/if}</td
              >
              <td class="needed"
                >{#each pkg.needed_by as harness (harness)}<span class="chip tag mono"
                    >{harnessLabel(harness).toLowerCase()}</span
                  >{/each}</td
              >
              {#each history.generations as generation, index (generation.generation)}
                {@const held = history.version(pkg.id, index)}
                <td
                  class="mono gen"
                  class:changed={history.changed(pkg.id, index)}
                  title={history.changed(pkg.id, index) ? 'Changed in this generation' : undefined}
                  >{held ?? '—'}</td
                >
              {/each}
              <td class="end">
                {#if label && listing.platform}
                  <button
                    class="button small"
                    class:chosen={change !== undefined}
                    onclick={(event) => {
                      event.stopPropagation();
                      ontoggle(pkg);
                    }}>{label}</button
                  >
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      {:else}
        <tbody>
          <tr
            ><td colspan={6 + history.generations.length} class="muted">No package matches.</td></tr
          >
        </tbody>
      {/each}
    </table>
  </section>
{/if}

<style>
  .packages-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }
  .packages-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 12px;
  }
  .c-name {
    width: 220px;
  }
  .c-version {
    width: 160px;
  }
  .c-action {
    width: 128px;
  }
  .c-origin {
    width: 90px;
  }
  .c-needed {
    width: 190px;
  }
  .alias {
    margin-left: 6px;
    font-size: 11px;
  }
  .needed {
    white-space: nowrap;
    overflow: hidden;
  }
  .needed .chip + .chip {
    margin-left: 3px;
  }
  td,
  th {
    text-align: left;
    padding: 0 12px;
    height: 30px;
    vertical-align: middle;
    border-bottom: 1px solid var(--line);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .packages-group th {
    height: 24px;
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    background: var(--surface-subtle);
  }
  .packages-group .count {
    font-weight: 400;
    margin-left: 4px;
  }
  .packages-row {
    cursor: pointer;
  }
  .packages-row:hover {
    background: var(--row-hover);
  }
  .packages-row.selected {
    background: var(--row-selected);
  }
  .packages-row.staged td:first-child {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  td.end {
    text-align: right;
    padding: 0 8px;
  }
  td.end .button {
    vertical-align: middle;
  }
  .name {
    font-weight: 600;
  }
  .arrow {
    margin: 0 4px;
    color: var(--muted);
  }
  .to.managed {
    color: var(--success);
  }
  .to.none {
    color: var(--error);
  }
  td > .chip {
    margin: 0;
    padding: 0 6px;
    font-size: 11px;
    line-height: 16px;
    vertical-align: 1px;
  }
  td > .tag {
    margin-left: 8px;
    font-weight: 400;
    font-family: inherit;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11px;
  }
  .shadowed {
    text-decoration: line-through;
  }
  .unknown {
    color: var(--warning);
  }
  .name :global(.glyph) {
    margin-right: 8px;
  }
  .gen-head {
    white-space: nowrap;
  }
  .gen-head > * {
    vertical-align: middle;
  }
  .gen-head :global(.glyph) {
    margin-left: 5px;
  }
  .restore {
    margin-left: 4px;
  }
  td.gen {
    color: var(--muted);
    font-size: 11px;
  }
  td.gen.changed {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--ink);
  }
</style>
