-- Adds a physical-database identity alongside the existing logical
-- repository_id. source_instance_id identifies one physical
-- agent-trace.db lineage; it is generated exactly once per database by
-- application code, never by SQL, and stays stable across reopen and
-- setup reruns. Existing/placeholder rows default to an empty string,
-- which application code recognizes as "not yet claimed" and replaces
-- through a concurrency-safe atomic claim.

ALTER TABLE repository_metadata
ADD COLUMN source_instance_id TEXT NOT NULL DEFAULT '';
