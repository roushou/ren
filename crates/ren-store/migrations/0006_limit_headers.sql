-- The provider's limit headers on an attempt's response head, as JSON: an
-- array of {name, value}, from the allowlist in ren_protocol::limits, with
-- a value that was not plain, or that read as a credential, withheld as
-- null. NULL when the head carried none, or came before this migration.
ALTER TABLE attempts ADD COLUMN limit_headers TEXT;
