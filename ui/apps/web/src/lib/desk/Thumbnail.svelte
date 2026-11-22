<!--
  An image as a framed thumbnail that opens it full size: in a sent
  prompt's row of frames, or in the composer's tray, where its name sits
  beside it. The frame is sized from script, since the page's CSP refuses
  inline style attributes.
-->
<script lang="ts">
  import { Frame, Gallery, type AttachmentRead, type Size } from '#lib/attachments.ts';
  import type { Attachment } from 'svelte/attachments';

  let {
    src,
    read,
    shown,
    loading = 'eager',
    onview,
  }: {
    src: string;
    read: AttachmentRead;
    /** Where it is shown: a sent prompt's row, or the composer's tray. */
    shown: 'sent' | 'tray';
    loading?: 'lazy' | 'eager';
    onview: () => void;
  } = $props();

  const fit = $derived((shown === 'sent' ? Frame.SENT : Frame.TRAY).fit(read));

  /** Sizes the frame. Set from script, since the page's CSP refuses inline
   * style attributes. */
  const framed =
    (size: Size): Attachment<HTMLElement> =>
    (element) => {
      element.style.width = `${size.width}px`;
      element.style.height = `${size.height}px`;
    };
</script>

<button
  type="button"
  class={shown === 'sent' ? 'sent-image' : 'tray-view'}
  title="{read.name} · {Gallery.describe(read)}"
  aria-label="View {read.name}"
  {@attach framed(fit.frame)}
  onclick={onview}
  ><img
    {src}
    alt={shown === 'sent' ? read.name : ''}
    width={fit.image.width}
    height={fit.image.height}
    {loading}
  /></button
>

<style>
  .sent-image {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
    font-size: 12px;
  }
  .sent-image,
  .tray-view {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    padding: 0;
    border-top: 0;
    border-left: 0;
    background: var(--surface-inset);
    cursor: zoom-in;
  }
  .sent-image img,
  .tray-view img {
    display: block;
  }
  .tray-view {
    height: 40px;
    border-right: 1px solid var(--line);
    border-bottom: 0;
  }
</style>
