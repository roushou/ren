// Boundary parsing for management API JSON. Decoders check shape once, so code
// past the boundary receives typed values and never re-checks them.

declare const brand: unique symbol;
/** A value with a distinct meaning, such as an identity, that plain types cannot express. */
export type Brand<T, B extends string> = T & { readonly [brand]: B };

declare const unknownLabel: unique symbol;
/** Label text this UI does not recognise, kept verbatim instead of being dropped. */
export type UnknownLabel = string & { readonly [unknownLabel]: true };
/** A label from a closed vocabulary, or the raw text a newer daemon sent. */
export type Stored<T extends string> = T | UnknownLabel;

/** The response did not have the expected shape. */
export class ShapeError extends Error {
  readonly path: string;
  constructor(path: string, expected: string) {
    super(`Ren sent an unexpected response (${path || 'body'}: expected ${expected}).`);
    this.path = path;
  }
}

declare const reads: unique symbol;
/**
 * Decodes JSON into `T`. `R` is the JSON it accepts: `string` for a string,
 * `T | null` once nullable, and so on, or `unknown` for a hand-written decoder
 * that says nothing. `object<View, Wire>` checks each field's `R` against the
 * type the daemon sends the field as, so a decoder that would refuse what is
 * sent (another type, or `null`) fails `check`.
 */
export type Decoder<T, R = unknown> = ((value: unknown, path: string) => T) & {
  readonly [reads]?: (sent: R) => void;
};

/** Any decoder of `T`, whatever JSON it accepts: for code that only runs one. */
export type AnyDecoder<T> = Decoder<T, never>;

export const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

export const string: Decoder<string, string> = (value, path) => {
  if (typeof value !== 'string') throw new ShapeError(path, 'a string');
  return value;
};

export const boolean: Decoder<boolean, boolean> = (value, path) => {
  if (typeof value !== 'boolean') throw new ShapeError(path, 'a boolean');
  return value;
};

/** A non-negative integer: counts, sizes, ordinals, and status codes. */
export const count: Decoder<number, number> = (value, path) => {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0)
    throw new ShapeError(path, 'a non-negative integer');
  return value;
};

export const nullable =
  <T, R>(decode: Decoder<T, R>): Decoder<T | null, R | null> =>
  (value, path) =>
    value === null ? null : decode(value, path);

export const array =
  <T, R>(decode: Decoder<T, R>): Decoder<T[], R[]> =>
  (value, path) => {
    if (!Array.isArray(value)) throw new ShapeError(path, 'an array');
    return value.map((item, index) => decode(item, `${path}[${index}]`));
  };

/**
 * `T`'s field decoders when every field of `T` is one of `W`'s, each one
 * accepting what `W` sends that field as; otherwise a type naming the fields
 * `W` lacks, so the mismatch is the error.
 */
type Fields<T, W> = [StrayFields<T, W>] extends [never]
  ? { [K in keyof T]-?: Decoder<T[K], K extends keyof W ? W[K] : unknown> }
  : { missing_on_wire: StrayFields<T, W> };

/** The fields of a decoded type `T` that its wire type `W` does not send. */
export type StrayFields<T, W> = Exclude<keyof T, keyof W>;

/**
 * Decode the listed fields; fields a newer daemon adds are ignored. `W` is the
 * daemon's generated wire type (`wire/`) when the decoded type `T` differs from
 * it (labels kept as `Stored`, ids branded, text parsed): every field `T` reads
 * must still be one the daemon sends.
 */
export const object =
  <T, W = T>(shape: Fields<T, W>): Decoder<T, W> =>
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    const result: Partial<T> = {};
    const fields = shape as { [K in keyof T]-?: Decoder<T[K], unknown> };
    for (const key of Object.keys(fields) as (keyof T & string)[]) {
      result[key] = fields[key](value[key], path ? `${path}.${key}` : key);
    }
    return result as T;
  };

/** The tags the variants of a wire union `W` carry in their `Tag` field. */
export type Tags<W, Tag extends PropertyKey> =
  W extends Record<Tag, infer K extends string> ? K : never;

/** The variant of a wire union `W` whose `Tag` is `K`. */
export type Variant<W, Tag extends PropertyKey, K> = Extract<W, Record<Tag, K>>;

/**
 * A union decoded by its `tag` field. `cases` has one decoder for each tag
 * the wire union `W` sends, accepting that variant, so a case written as
 * `object<View, Variant<W, Tag, K>>` checks the variant's field names and
 * types, and a tag the daemon adds fails `check` until it has a case.
 * `otherwise` reads anything else: a tag a newer daemon sends, or a value
 * that is not an object.
 */
export const tagged =
  <T, W, Tag extends string>(
    tag: Tag,
    cases: { [K in Tags<W, Tag>]: Decoder<T, Variant<W, Tag, K>> },
    otherwise: (value: unknown, path: string) => T,
  ): Decoder<T, W> =>
  (value, path) => {
    const found = isRecord(value) ? value[tag] : undefined;
    const known = cases as Record<string, AnyDecoder<T> | undefined>;
    const decode =
      typeof found === 'string' && Object.hasOwn(known, found) ? known[found] : undefined;
    return decode ? decode(value, path) : otherwise(value, path);
  };

/** A case's tag, written into the variant it decodes. */
export const tagValue =
  <const K extends string>(tag: K): Decoder<K, K> =>
  () =>
    tag;

/** `decode`, or `absent` when the field is left out. */
export const absentAs =
  <T, R>(decode: Decoder<T, R>, absent: T): Decoder<T, R | undefined> =>
  (value, path) =>
    value === undefined ? absent : decode(value, path);

/** Recognised labels narrow to their literal type; any other text is kept. */
export const label = <const T extends string>(values: readonly T[]): Decoder<Stored<T>, string> => {
  const known = new Set<string>(values);
  return (value, path) => {
    const text = string(value, path);
    return known.has(text) ? (text as T) : (text as UnknownLabel);
  };
};

/** Opaque text with its own type, such as a native identifier or a cursor. */
export const branded =
  <B extends string>(): Decoder<Brand<string, B>, string> =>
  (value, path) =>
    string(value, path) as Brand<string, B>;

/** A count with its own type, such as a timestamp or a sequence number. */
export const brandedCount =
  <B extends string>(): Decoder<Brand<number, B>, number> =>
  (value, path) =>
    count(value, path) as Brand<number, B>;

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** A Ren identity: a hyphenated UUID. */
export class Uuid<B extends string> {
  /** Parse untrusted text, such as a URL parameter. */
  parse(raw: string | null | undefined): Brand<string, B> | null {
    return raw && uuidPattern.test(raw) ? (raw.toLowerCase() as Brand<string, B>) : null;
  }
  readonly decode: Decoder<Brand<string, B>, string> = (value, path) => {
    const id = this.parse(string(value, path));
    if (id === null) throw new ShapeError(path, 'a UUID');
    return id;
  };
}

/**
 * The default arm of a switch over a `Stored` label. It type-checks only once
 * every known label has its own case, and returns the unrecognised text.
 */
export const unrecognised = (value: UnknownLabel): UnknownLabel => value;

/** Exhaustiveness check for switches over our own unions. */
export function assertNever(value: never): never {
  throw new Error(`Unhandled case: ${JSON.stringify(value)}`);
}
