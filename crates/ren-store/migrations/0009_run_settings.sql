-- A run's settings as it was started: the model (as its client names it),
-- the effort, and the approval policy. NULL is the client's own choice;
-- runs started before these were recorded read as NULL too.
ALTER TABLE runs ADD COLUMN model TEXT;
ALTER TABLE runs ADD COLUMN effort TEXT;
ALTER TABLE runs ADD COLUMN approvals TEXT;
