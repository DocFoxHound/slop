-- Maker signatures (SECURITY §5.3): each release file's GitHub artifact attestation, verified
-- by Slop before review and again by the client before installing.

alter table release_files add column attestation text;
-- slop_provenance::Provenance: who built the file, from which repo and commit.
alter table release_files add column provenance jsonb;
-- Why a file has no verified maker signature, shown to the maker and the reviewer.
alter table release_files add column provenance_problem text;
