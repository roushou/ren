-- Why the client sent each call, from its digest: `work`, or a client's own
-- check (`quota_probe`), which listings count with the bookkeeping calls.
-- NULL until the call's digest is derived.
ALTER TABLE call_digests ADD COLUMN purpose TEXT;
