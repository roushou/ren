// A modal dialog's opening, as an attachment: shown modally as it mounts,
// and focused where its first keystroke belongs.
import type { Attachment } from 'svelte/attachments';

export class Dialogs {
  /** Show the dialog modally as it mounts, and focus what `focus` names in
   * it; the browser's own choice (its first control) when none. */
  static modal(
    focus?: (dialog: HTMLDialogElement) => HTMLElement | null,
  ): Attachment<HTMLDialogElement> {
    return (dialog) => {
      dialog.showModal();
      focus?.(dialog)?.focus();
    };
  }
}
