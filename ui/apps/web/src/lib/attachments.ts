// Images sent with a prompt: what the daemon says each one is, the
// composer's tray of uploads (each one uploading, ready, or refused), and
// how a prompt in the conversation finds the thumbnails of what it sent.
// Pure: no browser APIs; tested in tests/attachments.test.ts.
import {
  Uuid,
  array,
  boolean,
  count,
  label,
  object,
  string,
  type Brand,
  type Decoder,
  type Stored,
} from './decode.ts';
import type { Attachment as WireAttachment } from './wire/Attachment.ts';
import type { Dimensions as WireDimensions } from './wire/Dimensions.ts';
import type { Image as WireImage } from './wire/Image.ts';
import type { Attached as WireAttached } from './wire/Attached.ts';
import type { AttachmentEntry as WireAttachmentEntry } from './wire/AttachmentEntry.ts';
import type { AttachmentsBody as WireAttachmentsBody } from './wire/AttachmentsBody.ts';
import type { KeptAttachment as WireKeptAttachment } from './wire/KeptAttachment.ts';

export type AttachmentId = Brand<string, 'AttachmentId'>;
export const AttachmentId = new Uuid<'AttachmentId'>();

export const IMAGE_FORMATS = ['png', 'jpeg', 'gif', 'webp'] as const;
export type ImageFormat = (typeof IMAGE_FORMATS)[number];

/** The media types an upload may have: the formats Ren takes. */
export const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'] as const;

/** What an attachment is, as the daemon read it from its bytes. */
export interface AttachmentRead {
  name: string;
  format: Stored<ImageFormat>;
  width: number;
  height: number;
  size: number;
  sha256: string;
}

const dimensions = object<{ width: number; height: number }, WireDimensions>({
  width: count,
  height: count,
});
const kind = object<
  { format: Stored<ImageFormat>; dimensions: { width: number; height: number } },
  WireImage
>({ format: label(IMAGE_FORMATS), dimensions });

export const attachmentRead: Decoder<AttachmentRead, WireAttachment> = (value, path) => {
  const read = object<
    {
      name: string;
      kind: { format: Stored<ImageFormat>; dimensions: { width: number; height: number } };
      size: number;
      sha256: string;
    },
    WireAttachment
  >({ name: string, kind, size: count, sha256: string })(value, path);
  return {
    name: read.name,
    format: read.kind.format,
    width: read.kind.dimensions.width,
    height: read.kind.dimensions.height,
    size: read.size,
    sha256: read.sha256,
  };
};

/** One of a session's attachments, as listed. */
export interface KeptAttachment {
  id: AttachmentId;
  read: AttachmentRead;
  sent: boolean;
}

export const uploaded = object<
  { attachment: { id: AttachmentId; read: AttachmentRead } },
  WireAttached
>({
  attachment: object<{ id: AttachmentId; read: AttachmentRead }, WireAttachmentEntry>({
    id: AttachmentId.decode,
    read: attachmentRead,
  }),
});

export const keptList = object<{ attachments: KeptAttachment[] }, WireAttachmentsBody>({
  attachments: array(
    object<KeptAttachment, WireKeptAttachment>({
      id: AttachmentId.decode,
      read: attachmentRead,
      sent: boolean,
    }),
  ),
});

/** Whether a session's client takes images with a prompt. */
export const IMAGE_TAKING = ['always', 'if_offered', 'never'] as const;
export type ImageTaking = (typeof IMAGE_TAKING)[number];

/** An image uploaded to a session, ready to go with a prompt. */
export interface UploadedImage {
  id: AttachmentId;
  read: AttachmentRead;
}

/** One upload in the composer's tray. */
export type Chip =
  | { state: 'uploading'; key: number; name: string }
  | { state: 'ready'; key: number; id: AttachmentId; read: AttachmentRead }
  | { state: 'refused'; key: number; name: string; reason: string };

/**
 * The images about to go with the next prompt. Immutable: each change
 * returns a new tray. Uploads keep the order they were added in.
 */
export class Tray {
  readonly chips: readonly Chip[];
  readonly #next: number;

  constructor(chips: readonly Chip[] = [], next = 0) {
    this.chips = chips;
    this.#next = next;
  }

  /** An upload of `name` started; its key names it until it ends. */
  adding(name: string): { tray: Tray; key: number } {
    const key = this.#next;
    return {
      tray: new Tray([...this.chips, { state: 'uploading', key, name }], key + 1),
      key,
    };
  }

  /** The upload `key` became attachment `id`. */
  uploaded(key: number, id: AttachmentId, read: AttachmentRead): Tray {
    return this.#replace(key, { state: 'ready', key, id, read });
  }

  /** The upload `key` was refused. */
  refused(key: number, reason: string): Tray {
    const chip = this.chips.find((one) => one.key === key);
    if (!chip) return this;
    const name = chip.state === 'ready' ? chip.read.name : chip.name;
    return this.#replace(key, { state: 'refused', key, name, reason });
  }

  without(key: number): Tray {
    return new Tray(
      this.chips.filter((chip) => chip.key !== key),
      this.#next,
    );
  }

  /** The attachments ready to send, in order. */
  ids(): AttachmentId[] {
    return this.ready().map((image) => image.id);
  }

  /** The images ready to send, in order. */
  ready(): UploadedImage[] {
    return this.chips.flatMap((chip) =>
      chip.state === 'ready' ? [{ id: chip.id, read: chip.read }] : [],
    );
  }

  /** `images` back in the tray, after the ones in it: a prompt not sent, taken back to edit. */
  restored(images: readonly UploadedImage[]): Tray {
    const known = new Set(this.ids());
    const fresh = images.filter((image) => !known.has(image.id));
    const chips: Chip[] = fresh.map((image, index) => ({
      state: 'ready',
      key: this.#next + index,
      id: image.id,
      read: image.read,
    }));
    return new Tray([...this.chips, ...chips], this.#next + chips.length);
  }

  /** Whether an upload is still running: the prompt waits for it. */
  uploading(): boolean {
    return this.chips.some((chip) => chip.state === 'uploading');
  }

  /** Sent: the ready ones leave; refused ones stay to be read. */
  sent(): Tray {
    return new Tray(
      this.chips.filter((chip) => chip.state === 'refused'),
      this.#next,
    );
  }

  #replace(key: number, chip: Chip): Tray {
    return new Tray(
      this.chips.map((one) => (one.key === key ? chip : one)),
      this.#next,
    );
  }
}

/** A session's attachments by content, so a prompt finds what it sent. */
export class Gallery {
  readonly #byHash: ReadonlyMap<string, AttachmentId>;

  constructor(kept: readonly KeptAttachment[] = []) {
    this.#byHash = new Map(kept.map((one) => [one.read.sha256, one.id]));
  }

  /** The attachment whose bytes these are, if the session has it. */
  find(read: AttachmentRead): AttachmentId | null {
    return this.#byHash.get(read.sha256) ?? null;
  }

  /** Where a session's attachment is read from, through this UI server. */
  static url(session: string, id: AttachmentId): string {
    return `/control/sessions/${session}/attachments/${id}`;
  }

  /** An image's size, as `640×480 · 1.2 MB`. */
  static describe(read: AttachmentRead): string {
    const size =
      read.size >= 1024 * 1024
        ? `${(read.size / (1024 * 1024)).toFixed(1)} MB`
        : `${Math.max(1, Math.round(read.size / 1024))} KB`;
    return `${read.width}×${read.height} · ${size}`;
  }

  /** Whether a file's media type is one Ren takes. */
  static takes(type: string): boolean {
    return (IMAGE_TYPES as readonly string[]).includes(type);
  }
}
