<script lang="ts">
  import type { RecordingId } from './api.ts';
  import { findingLink, type FindingLine } from './activity.ts';
  import Icon from './Icon.svelte';
  import Glyph from './Glyph.svelte';
  import { Marks } from './marks.ts';
  let {
    findings,
    recording,
    page,
  }: {
    findings: FindingLine[];
    /** The recording the findings' calls belong to. */
    recording: RecordingId;
    /** The page showing them, which decides where each finding links. */
    page: 'recording' | 'turn';
  } = $props();
</script>

<ul class="finding-list">
  {#each findings as line, index (index)}
    {@const link = findingLink(line, recording, page)}
    <li>
      <span class="finding-title"
        ><Glyph mark={Marks.ofTone(line.tone)} label={true} /><strong>{line.title}</strong></span
      >
      <span class="finding-detail">{line.detail}</span>
      {#if link}<a class="text-button" href={link.href}
          >{link.label} <Icon name="arrow" size={12} /></a
        >{/if}
    </li>
  {/each}
</ul>

<style>
  .finding-title {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 7px;
  }
  strong {
    font-weight: 600;
  }
</style>
