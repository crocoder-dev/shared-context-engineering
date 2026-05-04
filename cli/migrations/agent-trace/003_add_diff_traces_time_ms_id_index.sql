-- Optimize the post-commit intersection query:
--   SELECT ... FROM diff_traces WHERE time_ms >= ?1 ORDER BY time_ms ASC, id ASC
-- The composite index covers both the range filter and the sort order,
-- avoiding a full table scan and an extra sort pass.
CREATE INDEX IF NOT EXISTS idx_diff_traces_time_ms_id
ON diff_traces (time_ms, id);
