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
    super(`TGV sent an unexpected response (${path || 'body'}: expected ${expected}).`);
    this.path = path;
  }
}

export type Decoder<T> = (value: unknown, path: string) => T;

export const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

export const string: Decoder<string> = (value, path) => {
  if (typeof value !== 'string') throw new ShapeError(path, 'a string');
  return value;
};

export const boolean: Decoder<boolean> = (value, path) => {
  if (typeof value !== 'boolean') throw new ShapeError(path, 'a boolean');
  return value;
};

/** A non-negative integer: counts, sizes, ordinals, and status codes. */
export const count: Decoder<number> = (value, path) => {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < 0)
    throw new ShapeError(path, 'a non-negative integer');
  return value;
};

export const nullable =
  <T>(decode: Decoder<T>): Decoder<T | null> =>
  (value, path) =>
    value === null ? null : decode(value, path);

export const array =
  <T>(decode: Decoder<T>): Decoder<T[]> =>
  (value, path) => {
    if (!Array.isArray(value)) throw new ShapeError(path, 'an array');
    return value.map((item, index) => decode(item, `${path}[${index}]`));
  };

/** Decode the listed fields; fields a newer daemon adds are ignored. */
export const object =
  <T>(shape: { [K in keyof T]-?: Decoder<T[K]> }): Decoder<T> =>
  (value, path) => {
    if (!isRecord(value)) throw new ShapeError(path, 'an object');
    const result: Partial<T> = {};
    for (const key of Object.keys(shape) as (keyof T & string)[]) {
      result[key] = shape[key](value[key], path ? `${path}.${key}` : key);
    }
    return result as T;
  };

/** Recognised labels narrow to their literal type; any other text is kept. */
export const label = <const T extends string>(values: readonly T[]): Decoder<Stored<T>> => {
  const known = new Set<string>(values);
  return (value, path) => {
    const text = string(value, path);
    return known.has(text) ? (text as T) : (text as UnknownLabel);
  };
};

/** Opaque text with its own type, such as a native identifier or a cursor. */
export const branded =
  <B extends string>(): Decoder<Brand<string, B>> =>
  (value, path) =>
    string(value, path) as Brand<string, B>;

/** A count with its own type, such as a timestamp or a sequence number. */
export const brandedCount =
  <B extends string>(): Decoder<Brand<number, B>> =>
  (value, path) =>
    count(value, path) as Brand<number, B>;

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** A TGV identity: a hyphenated UUID. */
export class Uuid<B extends string> {
  /** Parse untrusted text, such as a URL parameter. */
  parse(raw: string | null | undefined): Brand<string, B> | null {
    return raw && uuidPattern.test(raw) ? (raw.toLowerCase() as Brand<string, B>) : null;
  }
  readonly decode: Decoder<Brand<string, B>> = (value, path) => {
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
