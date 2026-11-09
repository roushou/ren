-- Who made each call: the caller's kind, the configured client it proved to
-- be, and the Tailscale login the request carried. Calls recorded before this
-- migration keep NULL: their caller is unknown, not unidentified.
ALTER TABLE calls ADD COLUMN caller TEXT;
ALTER TABLE calls ADD COLUMN client TEXT;
ALTER TABLE calls ADD COLUMN tailscale_login TEXT;
CREATE INDEX calls_client ON calls(client, started_at DESC, id DESC) WHERE client IS NOT NULL;
