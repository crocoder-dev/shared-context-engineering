CREATE INDEX IF NOT EXISTS idx_diff_traces_time_ms_id
ON diff_traces (time_ms, id);
