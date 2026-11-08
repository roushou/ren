-- The wire format each call used, and the provider of the upstream entry each
-- attempt went to, which a fallback could make differ from its call's.
-- Before this migration every provider spoke exactly one wire format and every
-- fallback stayed within its call's provider, so both are filled exactly for
-- earlier rows. A provider label this version does not know leaves `wire`
-- NULL: unknown, not guessed.
ALTER TABLE calls ADD COLUMN wire TEXT;
UPDATE calls SET wire = CASE provider
    WHEN 'anthropic' THEN 'anthropic_messages'
    WHEN 'codex' THEN 'openai_responses'
    WHEN 'openai' THEN 'openai_responses'
END;
ALTER TABLE attempts ADD COLUMN provider TEXT;
UPDATE attempts SET provider = (SELECT provider FROM calls WHERE calls.id = attempts.call);
