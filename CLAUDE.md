# Claude guidance

[`AGENTS.md`](AGENTS.md) is this repository's contributor guidance and applies in
full. These rules add to it:

- Don't keep old versions (`_v2`, `_old`, commented-out code). Replace, don't
  append.
- Don't add compat shims or fallbacks for things that never shipped.
- Don't add a dependency that duplicates one already in use.
- Before defining a new data type (Rust or TS), search for an existing one
  modelling the same concept: similar names, overlapping fields. Extend or
  reuse it instead of creating a near-duplicate.
- Define a type in the component that owns the concept (usually the one that
  produces it or defines its rules), not where it is first used.
- Separate types for the same concept are allowed only across layers (domain,
  DB row, wire DTO, or a tolerant reader of another version's output), named
  by role, with one conversion in one place.
- No `types.rs` / `models.rs` / `common` dumping grounds.
