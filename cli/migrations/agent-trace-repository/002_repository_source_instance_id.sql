-- Adds the stable per-database-lineage source-instance identity alongside the
-- logical repository_id already stored in repository_metadata. This column is
-- additive to the 001 baseline: existing rows get an empty placeholder here,
-- and initialization code (RepositoryAgentTraceDb::verify_or_initialize_repository_metadata)
-- atomically fills it in exactly once. A migration can only add a fixed
-- default, so it must never invent a pseudo-UUID itself.
ALTER TABLE repository_metadata ADD COLUMN source_instance_id TEXT NOT NULL DEFAULT '';
