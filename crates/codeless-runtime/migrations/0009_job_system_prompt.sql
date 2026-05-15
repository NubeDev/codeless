-- Per-job system prompt composed from the selected persona at submit
-- time. NULL means "use the server's configured default"; a populated
-- column overrides the default for every stage in the job. Stored on
-- the row so reruns and resumes reproduce the prompt the user picked.

ALTER TABLE jobs ADD COLUMN system_prompt TEXT;
