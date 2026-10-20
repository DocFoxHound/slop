-- Files over GitHub's 2 GiB limit, released in parts (Slop Build splits them), built 2026-10-20.
-- [{"url", "file_name", "size", "sha256", "attestation"}] in order; size_bytes and sha256 on the
-- row are the joined file's. Null for a file in one piece.
alter table release_files add column parts jsonb;
