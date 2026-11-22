<!--
  Layer 4: a decision with consequences, over a dimmed page. It asks one
  question, shows what the answer changes, and keeps the page out of reach
  until it is answered or dismissed (Escape, or Cancel). The primary action
  goes last, on the right, and no answer has focus until the operator picks
  one.
-->
<script lang="ts">
  import { untrack, type Snippet } from 'svelte';

  let {
    title,
    eyebrow = null,
    onclose,
    children,
    actions,
  }: {
    /** The question it asks. */
    title: string;
    /** What it is about, above the question. */
    eyebrow?: string | null;
    /** Dismissed without an answer. */
    onclose: () => void;
    /** What the answer changes. */
    children: Snippet;
    /** The answers, the primary one last. */
    actions: Snippet;
  } = $props();

  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    untrack(() => {
      dialog?.showModal();
      // The browser would focus the first answer, and Enter would give it
      // unread: the dialog takes focus, so every answer is a choice.
      dialog?.focus();
    });
  });
</script>

<dialog
  bind:this={dialog}
  class="modal"
  tabindex="-1"
  aria-labelledby="modal-title"
  oncancel={(event) => {
    event.preventDefault();
    onclose();
  }}
>
  <header>
    {#if eyebrow}<span class="eyebrow">{eyebrow}</span>{/if}
    <h2 id="modal-title">{title}</h2>
  </header>
  <div class="body">{@render children()}</div>
  <footer>{@render actions()}</footer>
</dialog>

<style>
  .modal {
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    padding: 0;
    border: 1px solid var(--popover-border);
    border-radius: 0;
    background: var(--popover);
    color: var(--ink);
    box-shadow: var(--elev-modal);
    font-size: 13px;
  }
  .modal:focus {
    outline: none;
  }
  .modal::backdrop {
    background: var(--scrim);
  }
  header {
    padding: 16px 18px 12px;
    border-bottom: 1px solid var(--line);
  }
  .eyebrow {
    display: block;
    margin-bottom: 4px;
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
  }
  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 18px;
    overflow: auto;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--line);
  }
</style>
