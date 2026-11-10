-- Who made each call: the caller's kind, the named user it proved to be, and
-- the Tailscale login the request carried. Calls recorded before this
-- migration keep NULL: their caller is unknown, not unidentified.
ALTER TABLE calls ADD COLUMN caller TEXT;
ALTER TABLE calls ADD COLUMN user TEXT;
ALTER TABLE calls ADD COLUMN tailscale_login TEXT;
CREATE INDEX calls_user ON calls(user, started_at DESC, id DESC) WHERE user IS NOT NULL;
