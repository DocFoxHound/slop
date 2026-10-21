-- "Does Slop know this file?" (GET /v1/verify/{sha256}, the Slop open standard): looked up by hash.
create index release_files_sha256 on release_files (sha256) where sha256 is not null;
