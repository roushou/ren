/**
 * A table's columns, declared once: its header is drawn from them, and so
 * are the outline rows it shows while its data is on its way, so the two
 * never disagree about how many columns there are or how each is aligned.
 */
export interface Column {
  label: string;
  /** Right-aligned figures. */
  numeric?: boolean;
  /** A class the column's cells carry, such as a fixed width. */
  class?: string;
}
