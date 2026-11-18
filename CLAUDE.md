# Claude guidance

[`AGENTS.md`](AGENTS.md) is this repository's contributor guidance and applies in
full. These rules add to it:

- Don't keep old versions (`_v2`, `_old`, commented-out code). Replace, don't
  append.
- Don't add compat shims or fallbacks for things that never shipped.
- Don't add a dependency that duplicates one already in use.
