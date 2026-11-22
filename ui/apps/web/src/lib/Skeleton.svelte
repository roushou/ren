<!--
  Where a value will go while it is on its way: a flat bar in the value's
  own slot, sized like what usually fills it. A page draws its real frame
  (headings, labels, columns) and puts these only where data goes.
-->
<script lang="ts">
  let {
    width = '60%',
    size = 'text',
  }: {
    /** Any CSS width; text slots vary it so rows read as text, not stripes. */
    width?: string;
    /** A line of text, a large figure, or a block such as a chart. */
    size?: 'text' | 'figure' | 'block';
  } = $props();
</script>

<!-- Its width is set from script: the CSP refuses inline style in markup the
     server draws, and an unsized slot would stretch to its row. -->
<span
  class="skeleton {size}"
  {@attach (node) => {
    node.style.width = width;
  }}
  aria-hidden="true"
></span>

<style>
  .skeleton {
    display: inline-block;
    align-self: flex-start;
    width: 6em;
    max-width: 100%;
    height: 9px;
    vertical-align: middle;
    background: var(--surface-inset);
    animation: breathe 1.4s ease-in-out infinite;
  }
  .figure {
    height: 20px;
  }
  .block {
    display: block;
    width: 100%;
    height: 100%;
  }
  @keyframes breathe {
    50% {
      opacity: 0.55;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .skeleton {
      animation: none;
    }
  }
</style>
