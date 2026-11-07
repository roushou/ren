<script lang="ts">
  import type { SessionId } from './api.ts';
  import { findingLink, type FindingLine } from './activity.ts';
  import Icon from './Icon.svelte';
  let {
    findings,
    session,
    page,
  }: {
    findings: FindingLine[];
    /** The session the findings' calls belong to. */
    session: SessionId;
    /** The page showing them, which decides where each finding links. */
    page: 'session' | 'turn';
  } = $props();
</script>

<ul class="finding-list">
  {#each findings as line, index (index)}
    {@const link = findingLink(line, session, page)}
    <li>
      <span class="chip {line.tone}">{line.title}</span>
      <span class="finding-detail">{line.detail}</span>
      {#if link}<a class="text-button" href={link.href}
          >{link.label} <Icon name="arrow" size={12} /></a
        >{/if}
    </li>
  {/each}
</ul>
