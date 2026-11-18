// Query strings for the daemon's routes, built from the parameter types the
// gateway declares (`wire/`). A `Query<W>` is made only from `W`'s fields, so
// a parameter the gateway renames or removes fails `check` where the query
// is built, not at runtime.

/** A query string for a route whose parameters the gateway declares as `W`. */
export type Query<W> = URLSearchParams & { readonly __parameters: W };

/** What a parameter is written from: absent, null, and blank are left out. */
export type QueryValue = string | number | boolean | null | undefined;

/** The values a parameter the gateway types as `P` is written from. */
type ValueOf<P> = Exclude<P, undefined> | null | undefined;

/** `W`'s parameters, each written from a value of its own type. */
export type QueryFields<W> = { [K in keyof W]?: ValueOf<W[K]> };

/** `fields` as a query string; numbers are written whole. */
export const query = <W>(fields: QueryFields<W>): Query<W> => {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(fields) as [string, QueryValue][]) {
    if (value === undefined || value === null || value === '') continue;
    params.set(key, typeof value === 'number' ? String(Math.floor(value)) : String(value));
  }
  return params as Query<W>;
};

/** The parameters `keys` of `from` (such as a page's own URL) as `W`'s query. */
export const picked = <W>(from: URLSearchParams, keys: readonly (keyof W & string)[]): Query<W> =>
  query<W>(Object.fromEntries(keys.map((key) => [key, from.get(key)])) as QueryFields<W>);

/** `base` with `changes` applied: a value replaces, null removes. */
export const changed = <W>(base: Query<W>, changes: QueryFields<W>): Query<W> => {
  const params = new URLSearchParams(base);
  for (const [key, value] of Object.entries(changes) as [string, QueryValue][]) {
    if (value === undefined || value === null || value === '') params.delete(key);
    else params.set(key, typeof value === 'number' ? String(Math.floor(value)) : String(value));
  }
  return params as Query<W>;
};
